//! How many assistant tasks this computer runs well at once: each one is an
//! agent process with its own browsers and tools, so memory bounds it first
//! (about 1.5 GB a task after 16 GB for the system and the person's apps),
//! then the cores (three tasks a core, since most of a task's time is spent
//! waiting on its model). Always at least 2, at most [`MAX_PARALLEL`].

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

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

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
