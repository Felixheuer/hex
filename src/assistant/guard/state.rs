//! The guard's state on disk, shared by Agent-Desk and every task's guard process:
//!
//! - `leases/<window>.json`: who holds a window (task, pid and its start
//!   time, app). Created exclusively (`O_EXCL`) under `leases/.lock`, so two
//!   processes never both win; a lease whose process is gone (its pid dead,
//!   or now another process's) has lapsed and anyone may take it.
//! - `guard/paused/<task>`: present while the person has taken `task` over;
//!   `<task>.resumed` holds the time it was last handed back.
//! - `guard/hands-off/<task>.json`: present while `task` moves the real
//!   pointer or presses real keys.
//! - `guard/borrowed/<task>.json`: present while `task`'s guard has brought
//!   one of its windows in front of the person for a moment (`proxy::borrow`),
//!   with why and until when, so Agent-Desk shows the desk frame and its tab.
//! - `guard/here/<task>.json`: a Here task's window in front, written by
//!   Agent-Desk for each request, so a guard started ahead (in another window)
//!   serves the request's own window without new spawn arguments.
//! - `parking/<window>.json`: where a window Agent-Desk moved out of the person's
//!   way was before (`parking`), so it always goes back.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

/// A window a task's guard brought in front of the person for a moment,
/// as stored (`guard/borrowed/<task>.json`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BorrowFile {
    pub task: String,
    /// The guard's process and when it started: a borrow of a guard that
    /// ended (a crash) has lapsed.
    pub pid: u32,
    #[serde(default)]
    pub started: Option<u64>,
    pub window_id: String,
    pub app: String,
    /// Why (`guard::BORROW_IGNORED`, `guard::BORROW_KEYS`).
    pub why: String,
    /// When it began, and when it ends at the latest (Unix ms).
    pub at_ms: u64,
    pub until_ms: u64,
    /// The person's app in front before, which gets the front back.
    #[serde(default)]
    pub before: Option<String>,
}

impl BorrowFile {
    /// Whether its guard is still running.
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

    /// `<support>/assistant`, where the running Agent-Desk keeps it.
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

    fn borrowed_dir(&self) -> PathBuf {
        self.root.join("guard").join("borrowed")
    }

    fn borrowed_path(&self, task: &str) -> PathBuf {
        self.borrowed_dir()
            .join(format!("{}.json", file_name(task)))
    }

    /// Records `task`'s borrowed window (`None`: the front went back).
    pub fn set_borrowed(&self, task: &str, borrow: Option<&BorrowFile>) {
        let path = self.borrowed_path(task);
        match borrow.and_then(|borrow| serde_json::to_vec(borrow).ok()) {
            Some(bytes) => {
                let _ = std::fs::create_dir_all(self.borrowed_dir());
                // Whole or not at all: a reader never sees half of it.
                let partial = path.with_extension("partial");
                if std::fs::write(&partial, bytes).is_ok() {
                    let _ = std::fs::rename(&partial, &path);
                }
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Every window brought in front now, by guards still running.
    pub fn borrowed(&self) -> Vec<BorrowFile> {
        let mut borrows: Vec<BorrowFile> = json_files(&self.borrowed_dir())
            .into_iter()
            .filter_map(|path| serde_json::from_slice(&std::fs::read(path).ok()?).ok())
            .filter(BorrowFile::live)
            .collect();
        borrows.sort_by(|a, b| a.task.cmp(&b.task));
        borrows
    }

    fn here_path(&self, task: &str) -> PathBuf {
        self.root
            .join("guard")
            .join("here")
            .join(format!("{}.json", file_name(task)))
    }

    /// Records `task`'s Here window (`None` forgets it).
    pub fn set_here_window(&self, task: &str, window: Option<&super::WireWindow>) {
        let path = self.here_path(task);
        match window.and_then(|window| serde_json::to_vec(window).ok()) {
            Some(bytes) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// `task`'s Here window as Agent-Desk last recorded it.
    pub fn here_window(&self, task: &str) -> Option<super::WireWindow> {
        serde_json::from_slice(&std::fs::read(self.here_path(task)).ok()?).ok()
    }

    fn lease_path(&self, window_id: &str) -> PathBuf {
        self.leases_dir()
            .join(format!("{}.json", file_name(window_id)))
    }

    /// Runs `body` while holding the leases' lock, so a lapsed lease is
    /// taken over by one process only.
    pub(super) fn locked<T>(&self, body: impl FnOnce() -> T) -> T {
        locked_in(&self.leases_dir(), body)
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

    /// Hands `window_id` from `from` (which must hold it) to `to` in process
    /// `pid` (the desk, held by Agent-Desk): the lease outlives `from`'s process.
    pub fn hand_over(&self, window_id: &str, from: &str, to: &str, pid: u32, now_ms: u64) -> bool {
        self.locked(|| {
            let path = self.lease_path(window_id);
            let Some(lease) = read_lease(&path).filter(|lease| lease.task == from) else {
                return false;
            };
            let _ = std::fs::remove_file(&path);
            let handed = LeaseFile {
                task: to.to_owned(),
                pid,
                window_id: window_id.to_owned(),
                app: lease.app,
                since_ms: now_ms,
                started: started_us(pid),
            };
            create_exclusive(&path, &handed).is_ok()
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
        self.set_borrowed(task, None);
        self.set_here_window(task, None);
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
        for path in json_files(&self.borrowed_dir()) {
            let lapsed = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<BorrowFile>(&bytes).ok())
                .is_none_or(|borrow| !borrow.live());
            if lapsed {
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
            // The resume first, then the pause goes: a guard that sees the
            // pause end already reads this resume, not the one before.
            let _ = std::fs::write(
                dir.join(format!("{}.resumed", file_name(task))),
                now_ms.to_string(),
            );
            let _ = std::fs::remove_file(marker);
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
        HandsOn {
            path: Some(path),
            lock: None,
        }
    }

    /// Takes the real pointer and keyboard for `task`, one task at a time:
    /// waits up to `wait` while another task's action uses them, then
    /// gives up with that task's id. The hold is the operating system's
    /// lock on `held.lock`, so a holder that ended (a crash) gives it up by
    /// itself and two tasks can never both take it. The marker and the hold
    /// go when the value drops.
    pub fn take_hands(&self, task: &str, pid: u32, wait: Duration) -> Result<HandsOn, String> {
        use fs2::FileExt;
        let dir = self.hands_dir();
        let _ = std::fs::create_dir_all(&dir);
        let holder_path = dir.join("held.json");
        let Ok(lock) = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("held.lock"))
        else {
            return Ok(self.hands_on(task, pid));
        };
        let until = Instant::now() + wait;
        loop {
            if lock.try_lock_exclusive().is_ok() {
                let body =
                    serde_json::json!({ "task": task, "pid": pid, "started": started_us(pid) });
                let _ = std::fs::write(&holder_path, body.to_string());
                let mut hands = self.hands_on(task, pid);
                hands.lock = Some((lock, holder_path));
                return Ok(hands);
            }
            let holder = read_hands(&holder_path);
            // Ours already (an action inside an action): it goes on, and
            // leaves the outer hold's marker alone.
            if let Some((holder_task, holder_pid, _)) = &holder
                && holder_task == task
                && *holder_pid == pid
            {
                return Ok(HandsOn {
                    path: None,
                    lock: None,
                });
            }
            if Instant::now() >= until {
                return Err(holder.map(|(task, _, _)| task).unwrap_or_default());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
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

/// A hands-off marker, and the hold on the real input when it was taken
/// with [`Store::take_hands`]; removed (and let go) on drop.
pub struct HandsOn {
    path: Option<PathBuf>,
    /// The locked file and the note of who holds it.
    lock: Option<(File, PathBuf)>,
}

impl Drop for HandsOn {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
        if let Some((lock, holder)) = self.lock.take() {
            let _ = std::fs::remove_file(holder);
            let _ = fs2::FileExt::unlock(&lock);
        }
    }
}

/// Runs `body` while holding `dir/.lock` (created with `dir`), so
/// processes sharing `dir` take turns.
pub(super) fn locked_in<T>(dir: &Path, body: impl FnOnce() -> T) -> T {
    let _ = std::fs::create_dir_all(dir);
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

pub(super) fn json_files(dir: &Path) -> Vec<PathBuf> {
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

/// When process `pid` started, in µs since 1970: `proc_pidinfo` on macOS,
/// `/proc/<pid>/stat` and the boot time on Linux, `GetProcessTimes` on
/// Windows. The same process always gives the same value.
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
        if pid == 0 {
            return None;
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let boot = std::fs::read_to_string("/proc/stat").ok()?;
        // SAFETY: sysconf only reads a constant.
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        linux_started_us(&stat, &boot, u64::try_from(ticks).ok()?)
    }
    #[cfg(windows)]
    {
        windows::started_us(pid)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = pid;
        None
    }
}

/// The start time in a `/proc/<pid>/stat` line (its 22nd field, in clock
/// ticks since boot) plus `btime` from `/proc/stat`, in µs since 1970.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_started_us(stat: &str, proc_stat: &str, ticks_per_second: u64) -> Option<u64> {
    // The command name, in parentheses, may hold spaces and parentheses.
    let after_name = &stat[stat.rfind(')')? + 1..];
    // Fields from the 3rd (state) on: the start time is the 22nd field.
    let start_ticks: u64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let boot_seconds: u64 = proc_stat
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    if ticks_per_second == 0 {
        return None;
    }
    Some(boot_seconds * 1_000_000 + start_ticks * 1_000_000 / ticks_per_second)
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
        windows::alive(pid as u32)
    }
    #[cfg(not(any(unix, windows)))]
    {
        true
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, FILETIME, GetLastError, HANDLE, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// An open process handle, closed on drop.
    struct Process(HANDLE);

    impl Process {
        fn open(pid: u32) -> Option<Self> {
            // SAFETY: plain call; a null handle means it failed.
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            (!handle.is_null()).then_some(Self(handle))
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: the handle is open and closed only here.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Running: it opens and has no exit code yet. One Agent-Desk may not open
    /// (another user's) exists too.
    pub fn alive(pid: u32) -> bool {
        let Some(process) = Process::open(pid) else {
            // SAFETY: reads this thread's last error.
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        };
        let mut code = 0_u32;
        // SAFETY: the handle is open; `code` is written.
        let read = unsafe { GetExitCodeProcess(process.0, &mut code) } != 0;
        read && code == STILL_ACTIVE as u32
    }

    /// Its creation time, in µs since 1970.
    pub fn started_us(pid: u32) -> Option<u64> {
        let process = Process::open(pid)?;
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: the handle is open; each FILETIME is written.
        let read = unsafe {
            GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user)
        } != 0;
        if !read {
            return None;
        }
        // 100 ns ticks since 1601.
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        (ticks / 10).checked_sub(11_644_473_600_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linux_start_time_is_boot_time_plus_its_ticks() {
        let stat = "4242 (my (odd) name) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 \
                    12345 1000 10 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";
        let proc_stat = "cpu  1 2 3\nbtime 1794470000\nprocesses 99\n";
        assert_eq!(
            linux_started_us(stat, proc_stat, 100),
            Some(1_794_470_000_000_000 + 123_450_000)
        );
        assert_eq!(linux_started_us("garbage", proc_stat, 100), None);
    }

    #[test]
    fn this_process_is_alive_and_keeps_its_start_time() {
        let pid = std::process::id();
        assert!(alive(pid));
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            let started = started_us(pid);
            assert!(started.is_some());
            assert_eq!(started, started_us(pid));
            assert!(running(pid, started));
        }
    }
}
