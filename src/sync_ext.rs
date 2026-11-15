//! Locks that survive a panic elsewhere.
//!
//! A `Mutex` is poisoned when a thread panics while holding it. Agent-Desk's
//! shared state stays usable after such a panic (each holder leaves it
//! consistent between statements), so every lock takes the data either way.
//! `mutex.lock_ok()` says that once, instead of
//! `.lock().unwrap_or_else(|error| error.into_inner())` at every call.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// [`Mutex::lock`] that takes the data even when the lock was poisoned.
pub(crate) trait LockExt<T: ?Sized> {
    fn lock_ok(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> LockExt<T> for Mutex<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// [`RwLock::read`] and [`RwLock::write`] that take the data even when the
/// lock was poisoned.
#[allow(dead_code)]
pub(crate) trait RwLockExt<T: ?Sized> {
    fn read_ok(&self) -> RwLockReadGuard<'_, T>;
    fn write_ok(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T: ?Sized> RwLockExt<T> for RwLock<T> {
    fn read_ok(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_ok(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_poisoned_lock_still_gives_its_data() {
        let shared = Arc::new(Mutex::new(1));
        let held = shared.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(shared.is_poisoned());
        *shared.lock_ok() += 1;
        assert_eq!(*shared.lock_ok(), 2);

        let rw = Arc::new(RwLock::new(5));
        let held = rw.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.write().unwrap();
            panic!("poison it");
        })
        .join();
        *rw.write_ok() += 1;
        assert_eq!(*rw.read_ok(), 6);
    }
}
