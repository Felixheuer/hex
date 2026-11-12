//! The guard's state on disk, shared by Hex and every task's guard process:
//!
//! - `leases/<window>.json`: who holds a window (task, pid and its start
//!   time, app). Created exclusively (`O_EXCL`) under `leases/.lock`, so two
//!   processes never both win; a lease whose process is gone (its pid dead,
//!   or now another process's) has lapsed and anyone may take it.
//! - `guard/paused/<task>`: present while the person has taken `task` over;
//!   `<task>.resumed` holds the time it was last handed back.
//! - `guard/hands-off/<task>.json`: present while `task` moves the real
//!   pointer or presses real keys.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A lease as stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LeaseFile {
    pub task: String,
    pub pid: u32,
    pub window_id: String,
    pub app: String,
    pub since_ms: u64,
    /// When `pid` started (µs since 1970), so a reused pid isn't taken
    /// for the holder.
    #[serde(default)]
    pub started: Option<u64>,
}

impl LeaseFile {
    /// Whether its holder is still running.
    pub fn live(&self) -> bool {
        running(self.pid, self.started)
    }
}

/// What asking for a window came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Acquire {
    /// It's ours now (it was free, or its holder had died).
    Acquired,
    /// We held it already.
    Held,
    /// Another live task holds it.
    Taken(String),
}

/// The guard's folder (`<support>/assistant`).
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `<support>/assistant`, where the running Hex keeps it.
    pub fn standard() -> Option<Self> {
        crate::app_paths::support_dir()
            .ok()
            .map(|support| Self::new(support.join("assistant")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn leases_dir(&self) -> PathBuf {
        self.root.join("leases")
    }

    fn paused_dir(&self) -> PathBuf {
        self.root.join("guard").join("paused")
    }

    fn hands_dir(&self) -> PathBuf {
        self.root.join("guard").join("hands-off")
    }

    fn lease_path(&self, window_id: &str) -> PathBuf {
        self.leases_dir()
            .join(format!("{}.json", file_name(window_id)))
    }

    /// Runs `body` while holding the leases' lock, so a lapsed lease is
    /// taken over by one process only.
    fn locked<T>(&self, body: impl FnOnce() -> T) -> T {
        let dir = self.leases_dir();
        let _ = std::fs::create_dir_all(&dir);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"));
        let held = lock.as_ref().ok().filter(|file| file.lock().is_ok());
        let result = body();
        if let Some(file) = held {
            let _ = file.unlock();
        }
        result
    }

    /// Takes `window_id` for `task` (in process `pid`) unless another live
    /// task holds it. A lapsed lease (its pid is dead) is taken over; a lease
    /// of `task`'s own from an earlier process is adopted.
    pub fn acquire(
        &self,
        task: &str,
        pid: u32,
        window_id: &str,
        app: &str,
        now_ms: u64,
    ) -> Acquire {
        self.locked(|| {
            let path = self.lease_path(window_id);
            if let Some(lease) = read_lease(&path) {
                if lease.task == task && lease.pid == pid {
                    return Acquire::Held;
                }
                if lease.task != task && lease.live() {
                    return Acquire::Taken(lease.task);
                }
                // Lapsed, or ours from a process that has ended or restarted.
                let _ = std::fs::remove_file(&path);
            }
            let lease = LeaseFile {
                task: task.to_owned(),
                pid,
                window_id: window_id.to_owned(),
                app: app.to_owned(),
                since_ms: now_ms,
                started: started_us(pid),
            };
            match create_exclusive(&path, &lease) {
                Ok(()) => Acquire::Acquired,
                Err(_) => match read_lease(&path) {
                    Some(other) if other.task == task => Acquire::Held,
                    Some(other) => Acquire::Taken(other.task),
                    None => Acquire::Taken(String::new()),
                },
            }
        })
    }

    /// Who holds `window_id` now, if anyone live does.
    pub fn holder(&self, window_id: &str) -> Option<LeaseFile> {
        read_lease(&self.lease_path(window_id)).filter(LeaseFile::live)
    }

    /// Every live lease.
    pub fn leases(&self) -> Vec<LeaseFile> {
        let mut leases: Vec<LeaseFile> = self
            .lease_files()
            .into_iter()
            .filter_map(|path| read_lease(&path))
            .filter(LeaseFile::live)
            .collect();
        leases.sort_by(|a, b| (a.since_ms, &a.window_id).cmp(&(b.since_ms, &b.window_id)));
        leases
    }

    /// The windows `task` holds.
    pub fn held_by(&self, task: &str) -> Vec<LeaseFile> {
        let mut leases = self.leases();
        leases.retain(|lease| lease.task == task);
        leases
    }

    /// Gives up `window_id` if `task` holds it.
    pub fn release(&self, task: &str, window_id: &str) {
        self.locked(|| {
            let path = self.lease_path(window_id);
            if read_lease(&path).is_some_and(|lease| lease.task == task) {
                let _ = std::fs::remove_file(path);
            }
        });
    }

    /// Gives up every lease `task` holds.
    pub fn release_all(&self, task: &str) {
        self.locked(|| {
            for path in self.lease_files() {
                if read_lease(&path).is_some_and(|lease| lease.task == task) {
                    let _ = std::fs::remove_file(path);
                }
            }
        });
        let _ = std::fs::remove_file(self.hands_dir().join(format!("{}.json", file_name(task))));
    }

    /// Removes lapsed leases and hands-off markers (their process is gone).
    pub fn sweep(&self) {
        self.locked(|| {
            for path in self.lease_files() {
                let lapsed = match read_lease(&path) {
                    Some(lease) => !lease.live(),
                    None => is_old(&path),
                };
                if lapsed {
                    let _ = std::fs::remove_file(path);
                }
            }
        });
        for path in json_files(&self.hands_dir()) {
            if read_hands(&path).is_none_or(|(_, pid, started)| !running(pid, started)) {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn lease_files(&self) -> Vec<PathBuf> {
        json_files(&self.leases_dir())
    }

    /// Pauses (`true`) or resumes `task`.
    pub fn set_paused(&self, task: &str, paused: bool, now_ms: u64) {
        let dir = self.paused_dir();
        let _ = std::fs::create_dir_all(&dir);
        let marker = dir.join(file_name(task));
        if paused {
            let _ = std::fs::write(marker, now_ms.to_string());
        } else if marker.exists() {
            let _ = std::fs::remove_file(marker);
            let _ = std::fs::write(
                dir.join(format!("{}.resumed", file_name(task))),
                now_ms.to_string(),
            );
        }
    }

    /// What the person authorized `task` to do without asking
    /// (`authorized/<task>.json`, written by the brain); nothing when absent.
    pub fn authorized(&self, task: &str) -> crate::assistant::autonomy::Authorized {
        std::fs::read(
            self.root
                .join("authorized")
                .join(format!("{}.json", file_name(task))),
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
    }

    /// Forgets every pause (no task of an earlier run is still taken over).
    pub fn clear_paused(&self) {
        let _ = std::fs::remove_dir_all(self.paused_dir());
    }

    pub fn paused(&self, task: &str) -> bool {
        self.paused_dir().join(file_name(task)).exists()
    }

    /// When `task` was last handed back, as stored (changes on each resume).
    pub fn resumed(&self, task: &str) -> Option<String> {
        std::fs::read_to_string(
            self.paused_dir()
                .join(format!("{}.resumed", file_name(task))),
        )
        .ok()
    }

    /// Marks `task` as using real input until the returned value drops.
    pub fn hands_on(&self, task: &str, pid: u32) -> HandsOn {
        let dir = self.hands_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.json", file_name(task)));
        let body = serde_json::json!({ "task": task, "pid": pid, "started": started_us(pid) });
        let _ = std::fs::write(&path, body.to_string());
        HandsOn { path }
    }

    /// The task using real input right now, if any.
    pub fn hands_off(&self) -> Option<String> {
        json_files(&self.hands_dir())
            .into_iter()
            .filter_map(|path| read_hands(&path))
            .find(|(_, pid, started)| running(*pid, *started))
            .map(|(task, _, _)| task)
    }
}

/// A hands-off marker; removed on drop.
pub struct HandsOn {
    path: PathBuf,
}

impl Drop for HandsOn {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn read_lease(path: &Path) -> Option<LeaseFile> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn read_hands(path: &Path) -> Option<(String, u32, Option<u64>)> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some((
        value.get("task")?.as_str()?.to_owned(),
        u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        value.get("started").and_then(serde_json::Value::as_u64),
    ))
}

fn create_exclusive(path: &Path, lease: &LeaseFile) -> std::io::Result<()> {
    let mut file: File = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec(lease).map_err(std::io::Error::other)?)?;
    file.sync_all()
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect()
}

/// A file unreadable as a lease for over a few seconds (not one being
/// written right now).
fn is_old(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age.as_secs() > 5)
}

/// A window id or task id as a file name: letters, digits, `-`, `_`, `.`.
pub fn file_name(id: &str) -> String {
    let name: String = id
        .trim()
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    if name.is_empty() { "_".into() } else { name }
}

/// Whether process `pid` is running and is the one that started at
/// `started` (when known): a pid the system reused for another process
/// doesn't count.
pub fn running(pid: u32, started: Option<u64>) -> bool {
    alive(pid)
        && match (started, started_us(pid)) {
            (Some(recorded), Some(now)) => recorded == now,
            _ => true,
        }
}

/// When process `pid` started, in µs since 1970 (`proc_pidinfo` on macOS,
/// `/proc` on Linux, `GetProcessTimes` on Windows).
pub fn started_us(pid: u32) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        // SAFETY: `info` is a `proc_bsdinfo` of `size` bytes for the kernel
        // to fill.
        let wrote = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size)
        };
        (wrote == size).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let boot = std::fs::read_to_string("/proc/stat").ok()?;
        // SAFETY: Reads a constant of the system's.
        let ticks = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).ok()?;
        linux_started_us(&stat, &boot, ticks)
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
        use windows_sys::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: Opens `pid` for queries only, reads its times into locals,
        // and closes the handle.
        let read = unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let read = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
            CloseHandle(process);
            read
        };
        // FILETIME counts 100 ns since 1601.
        const UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
        let since_1601 =
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        (read != 0).then(|| since_1601.saturating_sub(UNIX_EPOCH_100NS) / 10)
    }
}

/// When a process started, in µs since 1970, from its `/proc/<pid>/stat`
/// (field 22: clock ticks after boot) and `/proc/stat`'s `btime` (boot, in
/// seconds since 1970), at `ticks` a second.
#[cfg(any(test, target_os = "linux"))]
pub(super) fn linux_started_us(stat: &str, boot: &str, ticks: u64) -> Option<u64> {
    // The name in parentheses may hold spaces: count after its end.
    let after_name = &stat[stat.rfind(')')? + 1..];
    let start_ticks: u64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let boot_seconds: u64 = boot
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    (ticks > 0).then(|| boot_seconds * 1_000_000 + start_ticks * 1_000_000 / ticks)
}

/// Whether process `pid` is running.
pub fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: Signal 0 only checks that the process exists.
        let found = unsafe { libc::kill(pid, 0) } == 0;
        found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        /// What GetExitCodeProcess answers while the process runs.
        const STILL_ACTIVE: u32 = 259;
        let mut code = 0_u32;
        // SAFETY: Opens `pid` for queries only, reads its exit code into a
        // local, and closes the handle.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
            if process.is_null() {
                // Gone, or not ours to ask (then it runs).
                return std::io::Error::last_os_error().raw_os_error() == Some(5);
            }
            let read = GetExitCodeProcess(process, &mut code);
            CloseHandle(process);
            read == 0 || code == STILL_ACTIVE
        }
    }
}
