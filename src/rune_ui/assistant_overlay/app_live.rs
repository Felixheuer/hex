//! Live pictures of the app windows a job works in, as Codex shows them: a
//! job's windows (`Task::windows`) hang off the screen while it works
//! (`assistant::space`, the guard parks what a background job opens), and
//! its card at the orb shows each one as a small live picture beside its
//! browsers. Pressing one opens it large in the overlay's card, where Take
//! over brings the window to the person and Hand back parks it again.
//!
//! ScreenCaptureKit takes a window's own content wherever it is, covered or
//! off the screen, so a parked window still shows. One thread per window
//! captures it while it shows: about a frame a second at a card's size, a
//! few at the window's own pixels while it's large; nothing while nothing
//! draws it. A window that leaves its job (released, or the job ended) is
//! brought back on the screen if it hangs off it.
//!
//! In the overlay's card and its strip a window is the session
//! `app:<window number>` ([`session`], [`number_of`]), beside the browsers'
//! agent-browser sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::assistant::TaskWindow;

/// The session prefix of an app window in the overlay's card.
pub(crate) const PREFIX: &str = "app:";
/// The long side a card's picture is captured at, before it's filtered down
/// to its thumbnail.
const CARD_EDGE: u32 = 1200;
/// The long side of the picture shown large: the window's own pixels on a
/// Retina display, up to this.
const LARGE_EDGE: u32 = 2560;
/// How often a window on a card is captured.
const CARD_EVERY: Duration = Duration::from_millis(1000);
/// How often the window shown large is captured.
const LARGE_EVERY: Duration = Duration::from_millis(350);
/// How often a window nothing draws looks whether it shows again.
const IDLE_POLL: Duration = Duration::from_millis(250);

/// One app window of a job and its latest picture.
pub(crate) struct AppPreview {
    pub(crate) window: TaskWindow,
    /// Its CoreGraphics window number.
    pub(crate) number: u32,
    /// The latest picture, filtered down to a thumbnail.
    pub(crate) pixels: rune::Pixels,
    /// The latest picture at the window's own pixels, while shown large.
    pub(crate) full: rune::Pixels,
    shown: AtomicBool,
    large: AtomicBool,
    failed: AtomicBool,
    stop: AtomicBool,
    /// Painted by a fixture ([`fake`]): no window behind it, so it is never
    /// captured or brought back, and stays until a fixture replaces it.
    fixture: bool,
}

impl AppPreview {
    fn new(window: TaskWindow, number: u32) -> Self {
        Self {
            window,
            number,
            pixels: rune::Pixels::new(),
            full: rune::Pixels::new(),
            shown: AtomicBool::new(false),
            large: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            fixture: false,
        }
    }

    /// Its session in the overlay's card.
    pub(crate) fn session(&self) -> String {
        session(self.number)
    }

    pub(crate) fn has_frame(&self) -> bool {
        self.pixels.size() != (0, 0)
    }

    /// Whether the last capture gave nothing (closed, minimized, or no
    /// Screen Recording permission).
    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// The sharpest picture there is.
    pub(crate) fn large_pixels(&self) -> &rune::Pixels {
        if self.full.size() != (0, 0) {
            &self.full
        } else {
            &self.pixels
        }
    }
}

/// The session of window `number` in the overlay's card.
pub(crate) fn session(number: u32) -> String {
    format!("{PREFIX}{number}")
}

/// The window number in an app window's session; `None` for a browser's.
pub(crate) fn number_of(session: &str) -> Option<u32> {
    session.strip_prefix(PREFIX)?.parse().ok()
}

#[derive(Default)]
struct Registry {
    /// Each live task's windows, in the order it got them.
    tasks: HashMap<String, Vec<Arc<AppPreview>>>,
    /// The jobs whose cards show at the orb.
    cards: Vec<String>,
    /// The jobs the live wall shows: their windows are captured too.
    wall: Vec<String>,
    /// The running jobs the Assistant window shows: their windows are
    /// captured while it's in front, so its job cards show them live.
    window: Vec<String>,
    /// The window shown large: (task, number).
    viewing: Option<(String, u32)>,
}

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// The windows worth a picture among `windows`: those agent-desktop named
/// (a window number), each once, in order.
pub(crate) fn pictured(windows: &[TaskWindow]) -> Vec<(TaskWindow, u32)> {
    let mut seen = Vec::new();
    let mut result = Vec::new();
    for window in windows {
        let Some(number) = window
            .window_id
            .as_deref()
            .and_then(crate::assistant::computer::window_number)
        else {
            continue;
        };
        if !seen.contains(&number) {
            seen.push(number);
            result.push((window.clone(), number));
        }
    }
    result
}

/// Of the windows `before` and `after` (task, number), those that left.
pub(crate) fn left(before: &[(String, u32)], after: &[(String, u32)]) -> Vec<(String, u32)> {
    before
        .iter()
        .filter(|window| !after.contains(window))
        .cloned()
        .collect()
}

/// Of the windows that `left`, those no task (nor the desk) has any more:
/// only they come back on the screen. One a quick turn handed to the desk
/// stays parked.
pub(crate) fn released(gone: &[(String, u32)], after: &[(String, u32)]) -> Vec<u32> {
    gone.iter()
        .map(|(_, number)| *number)
        .filter(|number| !after.iter().any(|(_, kept)| kept == number))
        .collect()
}

/// Every live task's windows (task, its windows). New windows get a
/// capture thread; windows that left stop theirs and come back on the
/// screen if they hang off it.
pub(crate) fn track(tasks: &[(String, Vec<TaskWindow>)]) {
    let mut registry = registry();
    let before: Vec<(String, u32)> = registry
        .tasks
        .iter()
        .flat_map(|(task, previews)| previews.iter().map(|p| (task.clone(), p.number)))
        .collect();
    let mut after = Vec::new();
    let mut next: HashMap<String, Vec<Arc<AppPreview>>> = HashMap::new();
    for (task, windows) in tasks {
        let old = registry.tasks.get(task);
        let previews: Vec<Arc<AppPreview>> = pictured(windows)
            .into_iter()
            .map(|(window, number)| {
                after.push((task.clone(), number));
                old.and_then(|old| old.iter().find(|p| p.number == number).cloned())
                    .unwrap_or_else(|| start(window, number))
            })
            .collect();
        if !previews.is_empty() {
            next.insert(task.clone(), previews);
        }
    }
    // A fixture's pictures stay as painted.
    for (task, previews) in &registry.tasks {
        if previews.iter().all(|preview| preview.fixture) {
            next.entry(task.clone()).or_insert_with(|| previews.clone());
            after.extend(
                previews
                    .iter()
                    .map(|preview| (task.clone(), preview.number)),
            );
        }
    }
    let gone = left(&before, &after);
    if gone.is_empty() && before.len() == after.len() {
        return;
    }
    let released = released(&gone, &after);
    for (task, number) in &gone {
        if let Some(preview) = registry
            .tasks
            .get(task)
            .and_then(|previews| previews.iter().find(|p| p.number == *number))
        {
            preview.stop.store(true, Ordering::Release);
            if released.contains(number) {
                bring_back(preview.window.clone());
            }
        }
    }
    registry.tasks = next;
    apply(&registry);
    drop(registry);
    super::live::notify();
}

/// The jobs whose cards show at the orb now: their windows are captured.
pub(crate) fn cards(tasks: &[String]) {
    let mut registry = registry();
    if registry.cards == tasks {
        return;
    }
    registry.cards = tasks.to_vec();
    apply(&registry);
}

/// The running jobs the Assistant window shows (empty: it's hidden or on
/// another pane): their windows are captured for its job cards.
pub(crate) fn window(tasks: &[String]) {
    let mut registry = registry();
    if registry.window == tasks {
        return;
    }
    registry.window = tasks.to_vec();
    apply(&registry);
}

/// The jobs the live wall shows (empty: it's closed): their windows are
/// captured while it shows.
pub(crate) fn wall(tasks: &[String]) {
    let mut registry = registry();
    if registry.wall == tasks {
        return;
    }
    registry.wall = tasks.to_vec();
    apply(&registry);
}

/// The overlay's card shows window `number` of `task` large (`None`:
/// nothing).
pub(crate) fn view(target: Option<(&str, u32)>) {
    let mut registry = registry();
    let target = target.map(|(task, number)| (task.to_owned(), number));
    if registry.viewing == target {
        return;
    }
    registry.viewing = target;
    apply(&registry);
}

/// `task`'s windows, while it works.
pub(crate) fn previews(task: &str) -> Vec<Arc<AppPreview>> {
    registry().tasks.get(task).cloned().unwrap_or_default()
}

/// The playground's stories, renders, and layout tests: `task` works in `windows` (app, number), each
/// showing the picture `paint` draws for its index.
pub(crate) fn fake(
    task: &str,
    windows: &[(&str, u32)],
    paint: impl Fn(usize) -> (u32, u32, Vec<u8>),
) {
    let previews = windows
        .iter()
        .enumerate()
        .map(|(index, (app, number))| {
            let mut preview = AppPreview::new(
                TaskWindow {
                    app: (*app).into(),
                    window_id: Some(format!("w-{number}")),
                },
                *number,
            );
            preview.fixture = true;
            let (width, height, rgba) = paint(index);
            preview.full.set(width, height, rgba.clone());
            preview.pixels.set(width, height, rgba);
            Arc::new(preview)
        })
        .collect();
    registry().tasks.insert(task.into(), previews);
}

/// Window `number` of `task`, while it works.
pub(crate) fn preview(task: &str, number: u32) -> Option<Arc<AppPreview>> {
    previews(task).into_iter().find(|p| p.number == number)
}

/// Take over: brings window `number` of `task` to the person, in front.
pub(crate) fn show(task: &str, number: u32) {
    if let Some(preview) = preview(task, number) {
        let window = preview.window.clone();
        spawn("hex-app-show", move || {
            if let Err(error) = crate::assistant::space::show_now(&window) {
                tracing::warn!(
                    error,
                    app = window.app,
                    "couldn't bring a job's window over"
                );
            }
        });
    }
}

/// Hand back: parks window `number` of `task` off the screen again.
pub(crate) fn park(task: &str, number: u32) {
    if let Some(preview) = preview(task, number) {
        let window = preview.window.clone();
        spawn("hex-app-park", move || {
            if let Err(error) = crate::assistant::space::park_now(&window) {
                tracing::warn!(error, app = window.app, "couldn't park a job's window");
            }
        });
    }
}

/// A window that left its job: back on the screen if it hangs off it.
fn bring_back(window: TaskWindow) {
    spawn("hex-app-back", move || {
        if let Err(error) = crate::assistant::space::bring_back(&window) {
            tracing::debug!(error, app = window.app, "a job's window stays where it is");
        }
    });
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.into()).spawn(work) {
        tracing::warn!(%error, "couldn't start a window thread");
    }
}

/// Which windows each capture thread takes, from what shows.
fn apply(registry: &Registry) {
    for (task, previews) in &registry.tasks {
        let carded = registry.cards.contains(task)
            || registry.wall.contains(task)
            || registry.window.contains(task);
        for preview in previews {
            let large = registry
                .viewing
                .as_ref()
                .is_some_and(|(viewed, number)| viewed == task && *number == preview.number);
            preview.large.store(large, Ordering::Release);
            preview.shown.store(carded || large, Ordering::Release);
        }
    }
}

/// A window's picture and its capture thread.
fn start(window: TaskWindow, number: u32) -> Arc<AppPreview> {
    let preview = Arc::new(AppPreview::new(window, number));
    let runner = preview.clone();
    spawn("hex-app-live", move || run(&runner));
    preview
}

/// Captures `preview`'s window while it shows, until it stops.
fn run(preview: &AppPreview) {
    while !preview.stop.load(Ordering::Acquire) {
        if !preview.shown.load(Ordering::Acquire) {
            std::thread::sleep(IDLE_POLL);
            continue;
        }
        let large = preview.large.load(Ordering::Acquire);
        let edge = if large { LARGE_EDGE } else { CARD_EDGE };
        match scrawl_capture::capture::capture_window(preview.number, edge) {
            Some((width, height, mut rgba)) => {
                // Where the agent just acted in it, as a ring.
                crate::assistant::input::pointer::stamp_recent(
                    preview.number,
                    &mut rgba,
                    width,
                    height,
                );
                let first = !preview.has_frame();
                let (tw, th, thumb) = super::live::fit_width(
                    &rgba,
                    width as usize,
                    height as usize,
                    super::live::THUMB_WIDTH,
                );
                preview.pixels.set(tw as u32, th as u32, thumb);
                if large {
                    preview.full.set(width, height, rgba);
                }
                if preview.failed.swap(false, Ordering::AcqRel) || first {
                    super::live::notify();
                }
            }
            None => {
                if !preview.failed.swap(true, Ordering::AcqRel) {
                    super::live::notify();
                }
            }
        }
        // Wait out the round, waking early when it's shown large.
        let every = if large { LARGE_EVERY } else { CARD_EVERY };
        let mut waited = Duration::ZERO;
        while waited < every && !preview.stop.load(Ordering::Acquire) {
            if !large && preview.large.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(IDLE_POLL.min(every - waited));
            waited += IDLE_POLL;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(app: &str, id: Option<&str>) -> TaskWindow {
        TaskWindow {
            app: app.into(),
            window_id: id.map(str::to_owned),
        }
    }

    #[test]
    fn a_fixture_s_windows_stay_when_no_task_has_them() {
        let task = "app-live-fixture-test";
        fake(task, &[("Numbers", 4_242)], |_| (1, 1, vec![0; 4]));
        // No task works in any window now: nothing is captured or brought
        // back, and the painted one stays.
        track(&[]);
        let kept = previews(task);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].fixture && !kept[0].stop.load(Ordering::Acquire));
    }

    #[test]
    fn a_window_is_a_session_beside_the_browsers() {
        assert_eq!(session(14569), "app:14569");
        assert_eq!(number_of("app:14569"), Some(14569));
        assert_eq!(number_of("hex-t"), None);
        assert_eq!(number_of("app:x"), None);
    }

    #[test]
    fn only_named_windows_get_a_picture_each_once() {
        let windows = [
            window("Mail", Some("w-12")),
            window("Notes", None),
            window("Mail", Some("w-12")),
            window("Numbers", Some("w-40")),
        ];
        let pictured: Vec<(String, u32)> = pictured(&windows)
            .into_iter()
            .map(|(window, number)| (window.app, number))
            .collect();
        assert_eq!(pictured, [("Mail".into(), 12), ("Numbers".into(), 40)]);
    }

    #[test]
    fn windows_that_leave_a_job_are_the_ones_brought_back() {
        let before = [("job".to_owned(), 12), ("job".to_owned(), 40)];
        let after = [("job".to_owned(), 40), ("other".to_owned(), 7)];
        assert_eq!(left(&before, &after), [("job".to_owned(), 12)]);
        assert!(left(&after, &after).is_empty());
    }

    #[test]
    fn a_window_handed_to_the_desk_stays_parked() {
        // The quick turn ended; its WhatsApp window (77) is the desk's now.
        let before = [("turn".to_owned(), 77), ("turn".to_owned(), 12)];
        let after = [("desk".to_owned(), 77)];
        let gone = left(&before, &after);
        assert_eq!(gone.len(), 2);
        assert_eq!(released(&gone, &after), [12]);
    }
}
