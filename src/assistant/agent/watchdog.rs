//! A quick turn that makes no progress doesn't sit at "Thinking…" until its
//! 30-minute limit: when its backend reports nothing (no stream line, no
//! tool call or result) for [`HERE_INACTIVITY`], its processes end
//! (`Control::stall`). The turn then goes on with the strong model once
//! (`agent::run_in`, as a fast model's hand-over does), and fails with a
//! Try again if that stalls too.
//!
//! OWNER: brain.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app_settings::AssistantBackend;
use crate::assistant::provider::{Control, Run};

/// How long a quick turn may report nothing before it counts as stuck.
pub const HERE_INACTIVITY: Duration = Duration::from_secs(45);

/// What a quick turn that stalled, and stalled again with the strong
/// model, ends with.
pub const STALLED: &str =
    "It got stuck: nothing happened for 45 seconds, so Hex stopped it. Try again.";

/// The inactivity limit of `run`: a quick turn on a backend Hex runs as a
/// process (the Claude API's requests have their own timeouts); `None`
/// for anything else.
pub fn limit(run: &Run) -> Option<Duration> {
    (run.quick && run.backend != AssistantBackend::ClaudeApi).then_some(HERE_INACTIVITY)
}

/// Runs `body`, handing it a `touch` to call on every sign of progress;
/// whenever `limit` passes without one, the turn's processes end
/// (`Control::stall`) and the clock starts again. No limit: just `body`.
pub fn watch<T>(
    control: &Control,
    limit: Option<Duration>,
    body: impl FnOnce(&dyn Fn()) -> T,
) -> T {
    let Some(limit) = limit else {
        return body(&|| {});
    };
    let last = Mutex::new(Instant::now());
    let done = AtomicBool::new(false);
    let touch = || *last.lock().unwrap_or_else(|error| error.into_inner()) = Instant::now();
    let tick = (limit / 20).clamp(Duration::from_millis(5), Duration::from_millis(500));
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                std::thread::sleep(tick);
                if control.cancelled() {
                    return;
                }
                let mut last = last.lock().unwrap_or_else(|error| error.into_inner());
                if last.elapsed() >= limit && !done.load(Ordering::Acquire) {
                    tracing::warn!(
                        seconds = limit.as_secs(),
                        "a quick turn made no progress; ending its backend"
                    );
                    control.stall();
                    *last = Instant::now();
                }
            }
        });
        // Set however `body` ends (a panic too), so the watcher ends with it.
        let _done = Done(&done);
        body(&touch)
    })
}

/// Sets its flag when dropped.
struct Done<'a>(&'a AtomicBool);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_without_progress_stalls_and_one_with_it_doesn_t() {
        let control = Control::default();
        watch(&control, Some(Duration::from_millis(40)), |_| {
            std::thread::sleep(Duration::from_millis(150));
        });
        assert!(control.stalls() >= 1);
        assert!(!control.cancelled(), "the task itself goes on");

        let busy = Control::default();
        watch(&busy, Some(Duration::from_millis(80)), |touch| {
            for _ in 0..15 {
                touch();
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        assert_eq!(busy.stalls(), 0);

        let unwatched = Control::default();
        let answer = watch(&unwatched, None, |_| {
            std::thread::sleep(Duration::from_millis(20));
            42
        });
        assert_eq!((answer, unwatched.stalls()), (42, 0));
    }

    #[test]
    fn only_quick_turns_on_process_backends_are_watched() {
        let mut run = Run::fixture(AssistantBackend::ClaudeCode);
        run.quick = true;
        assert_eq!(limit(&run), Some(HERE_INACTIVITY));
        run.quick = false;
        assert_eq!(limit(&run), None);
        let mut api = Run::fixture(AssistantBackend::ClaudeApi);
        api.quick = true;
        assert_eq!(limit(&api), None);
    }
}
