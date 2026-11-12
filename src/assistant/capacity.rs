//! How many assistant tasks this computer runs well at once: each one is an
//! agent process with its own browsers and tools, so memory bounds it first
//! (about 1.5 GB a task after 16 GB for the system and the person's apps),
//! then the cores (three tasks a core, since most of a task's time is spent
//! waiting on its model). Always at least 2, at most [`MAX_PARALLEL`].
//!
//! That number is a ceiling, not a promise: the browsers a job opens weigh
//! far more than its agent. So before each queued Background task starts,
//! the runner asks how tight memory is now ([`pressure`], the kernel's own
//! verdict on macOS) and [`background_room`] holds new ones back while it's
//! tight; they start by themselves once it eases, and one always runs.

use std::sync::OnceLock;

/// The most tasks Settings offers at once.
pub const MAX_PARALLEL: u32 = 50;

/// What a computer without readable hardware gets.
const FALLBACK: u32 = 8;

/// The recommendation for `ram_bytes` of memory and `cores` cores.
pub fn recommended_parallel_for(ram_bytes: u64, cores: u32) -> u32 {
    let ram_gb = ram_bytes as f64 / (1u64 << 30) as f64;
    let by_memory = ((ram_gb - 16.0) / 1.5).max(0.0);
    let by_cores = f64::from(cores) * 3.0;
    let recommended = by_memory.min(by_cores).floor() as u32;
    recommended.clamp(2, MAX_PARALLEL)
}

/// The recommendation for this computer, read once.
pub fn recommended_parallel() -> u32 {
    static RECOMMENDED: OnceLock<u32> = OnceLock::new();
    *RECOMMENDED.get_or_init(|| match hardware() {
        Some((ram, cores)) => recommended_parallel_for(ram, cores),
        None => FALLBACK,
    })
}

/// Memory in bytes and logical cores, from `hw.memsize` and `hw.ncpu`.
#[cfg(target_os = "macos")]
fn hardware() -> Option<(u64, u32)> {
    fn read<T: Default + Copy>(name: &std::ffi::CStr) -> Option<T> {
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>();
        // SAFETY: `value` is a plain integer of `size` bytes, and the name is
        // NUL-terminated; sysctlbyname writes at most `size` bytes.
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&raw mut value).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == std::mem::size_of::<T>()).then_some(value)
    }
    let ram = read::<u64>(c"hw.memsize")?;
    let cores = read::<i32>(c"hw.ncpu")?;
    Some((ram, u32::try_from(cores).ok()?))
}

/// Linux reads `/proc/meminfo`; elsewhere there's no cheap reading.
#[cfg(not(target_os = "macos"))]
fn hardware() -> Option<(u64, u32)> {
    let cores = std::thread::available_parallelism().ok()?.get() as u32;
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = meminfo
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        Some((kb * 1024, cores))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cores;
        None
    }
}

/// How tight memory is now.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Pressure {
    #[default]
    Normal,
    /// The system compresses or swaps: no more than two Background tasks
    /// start.
    Warning,
    /// It's about to kill apps: none starts while one runs.
    Critical,
}

/// Whether another Background task may start with `running` of them going,
/// at most `limit` (`max_parallel`), under `pressure`.
pub fn background_room(limit: usize, running: usize, pressure: Pressure) -> bool {
    let limit = match pressure {
        Pressure::Normal => limit,
        Pressure::Warning => limit.min(2),
        Pressure::Critical => 1,
    };
    running < limit.max(1)
}

/// How tight memory is now, read at most every two seconds.
pub fn pressure() -> Pressure {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<(Instant, Pressure)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((at, pressure)) = *last
        && at.elapsed() < Duration::from_secs(2)
    {
        return pressure;
    }
    let pressure = read_pressure().unwrap_or_default();
    if pressure != Pressure::Normal {
        tracing::info!(?pressure, "memory is tight: queued tasks wait");
    }
    *last = Some((Instant::now(), pressure));
    pressure
}

/// The kernel's memory pressure level (`kern.memorystatus_vm_pressure_level`:
/// 1 normal, 2 warning, 4 critical), as Activity Monitor shows it.
#[cfg(target_os = "macos")]
fn read_pressure() -> Option<Pressure> {
    let mut level: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    // SAFETY: `level` is a plain i32 of `size` bytes and the name is
    // NUL-terminated; sysctlbyname writes at most `size` bytes.
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            (&raw mut level).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0).then_some(match level {
        4 => Pressure::Critical,
        2 => Pressure::Warning,
        _ => Pressure::Normal,
    })
}

/// Linux: how much of the memory is still available (`MemAvailable`).
#[cfg(target_os = "linux")]
fn read_pressure() -> Option<Pressure> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    pressure_of_meminfo(&meminfo)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_pressure() -> Option<Pressure> {
    None
}

/// The pressure `/proc/meminfo` shows: under 5% available is critical,
/// under 12% a warning.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pressure_of_meminfo(meminfo: &str) -> Option<Pressure> {
    let field = |name: &str| -> Option<f64> {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    if total <= 0.0 {
        return None;
    }
    let share = available / total;
    Some(if share < 0.05 {
        Pressure::Critical
    } else if share < 0.12 {
        Pressure::Warning
    } else {
        Pressure::Normal
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn tight_memory_holds_new_background_tasks_back_but_one_always_runs() {
        assert!(background_room(10, 9, Pressure::Normal));
        assert!(!background_room(10, 10, Pressure::Normal));
        assert!(background_room(10, 1, Pressure::Warning));
        assert!(!background_room(10, 2, Pressure::Warning));
        assert!(background_room(10, 0, Pressure::Critical), "one always runs");
        assert!(!background_room(10, 1, Pressure::Critical));
        assert!(background_room(0, 0, Pressure::Normal), "a limit of 0 is 1");
    }

    #[test]
    fn meminfo_tells_the_pressure() {
        let info = |available: u64| {
            format!("MemTotal:       16000000 kB\nMemFree: 1 kB\nMemAvailable:   {available} kB\n")
        };
        assert_eq!(pressure_of_meminfo(&info(8_000_000)), Some(Pressure::Normal));
        assert_eq!(pressure_of_meminfo(&info(1_500_000)), Some(Pressure::Warning));
        assert_eq!(pressure_of_meminfo(&info(500_000)), Some(Pressure::Critical));
        assert_eq!(pressure_of_meminfo("nonsense"), None);
    }

    #[test]
    fn memory_then_cores_bound_the_recommendation() {
        assert_eq!(recommended_parallel_for(64 * GB, 18), 32);
        assert_eq!(recommended_parallel_for(32 * GB, 10), 10);
        assert_eq!(recommended_parallel_for(16 * GB, 8), 2, "never under two");
        assert_eq!(recommended_parallel_for(8 * GB, 8), 2);
        assert_eq!(recommended_parallel_for(128 * GB, 4), 12, "three a core");
        assert_eq!(recommended_parallel_for(512 * GB, 32), MAX_PARALLEL);
    }

    #[test]
    fn this_computer_gets_a_choosable_number() {
        let recommended = recommended_parallel();
        assert!((2..=MAX_PARALLEL).contains(&recommended));
    }
}
