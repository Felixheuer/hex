//! Live previews of a task's browsers: while the pointer rests on a task in
//! the orb's list, each of its agent-browser sessions streams its viewport
//! into a small picture ([`hover`]); pressing one opens it large right in
//! the overlay, and "Show in Assistant" there opens it large in the Assistant pane
//! ([`request_view`], [`view`]); in either, Take over lets the person
//! drive it ([`send`]).
//!
//! Both large views share one stream per browser: [`view`] says which
//! viewer shows what, and a browser any viewer shows large streams large.
//!
//! Every working task's browsers are listed from the moment they open
//! ([`track`]); those a job's card shows at the orb ([`cards`], a card's first
//! four) stream slowly (a frame a second), so a picture is already there when
//! the pointer comes; the hovered task's at a few frames a second, the one
//! viewed large at full size and fast. Every other browser streams nothing
//! ([`Rate::Idle`]) until it shows, so fifty jobs with a dozen browsers each
//! don't decode hundreds of pictures a second nobody sees. Each browser keeps only its latest
//! frame: a thumbnail filtered down to what the list draws (never below its
//! on-screen pixels), and, while hovered or viewed, the full frame. A
//! browser that becomes hovered or viewed with no full frame yet reconnects
//! at once, since a still page sends no new frame by itself. One thread lists
//! every tracked task's browsers (one agent-browser call a round) and one
//! thread per browser reads its stream (`assistant::browser::stream`). A
//! light Lightpanda browser draws no pixels; its card shows the page's title
//! and address instead.
//!
//! Views read [`previews`] and [`preview`]; frames wake whatever draws
//! their `rune::Pixels`, and a browser coming or going, or its page
//! changing, calls the hooks set with [`on_change`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

mod drive;
mod frames;

use crate::assistant::browser::stream::{self, Input, Message, Writer};
pub(crate) use drive::{key_input, to_page};
pub(super) use frames::fit_width;
use frames::run;

/// A thumbnail's width in pixels: the list draws it 148 points wide, so
/// this covers a Retina (2×) display with room, filtered down from the
/// frame rather than scaled by the GPU.
pub(super) const THUMB_WIDTH: usize = 360;
/// How often the tracked tasks' browsers are listed again.
const RELIST: Duration = Duration::from_millis(1500);

/// How fast a browser streams.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Rate {
    /// Nothing draws it (its job has no card showing, or it's past a card's
    /// four browsers): no stream at all, so fifty jobs with a dozen
    /// browsers each cost nothing until they show.
    Idle = 0,
    /// Its job's card shows it: a frame a second, so a picture is ready
    /// the moment the pointer comes.
    Background = 1,
    /// The live wall shows it among all of a job's browsers: two frames a
    /// second, at full size so a tile stays sharp.
    Wall = 2,
    /// Its task is hovered in the list: thumbnails.
    Small = 3,
    /// A viewer shows it large.
    Large = 4,
}

impl Rate {
    pub(crate) fn fps(self) -> u32 {
        match self {
            Self::Idle => 0,
            Self::Background => 1,
            Self::Wall => 2,
            Self::Small => 5,
            Self::Large => 24,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            4 => Self::Large,
            3 => Self::Small,
            2 => Self::Wall,
            1 => Self::Background,
            _ => Self::Idle,
        }
    }
}

/// How often an idle browser's thread looks whether it shows again.
const IDLE_POLL: Duration = Duration::from_millis(200);
/// How long a stream that failed waits before trying again.
const RETRY: Duration = Duration::from_secs(2);

/// One browser's live picture and what its page is.
pub(crate) struct Preview {
    /// The agent-browser session.
    pub(crate) session: String,
    /// What the agent calls it: "" for the task's main browser, else its
    /// name ("a", "suppliers-2").
    pub(crate) name: String,
    /// The latest frame, filtered down to a thumbnail ([`THUMB_WIDTH`]).
    pub(crate) pixels: rune::Pixels,
    /// The latest frame at full size, kept while hovered or viewed large.
    pub(crate) full: rune::Pixels,
    page: Mutex<Page>,
    writer: Mutex<Option<Writer>>,
    /// The [`Rate`] it streams at.
    rate: AtomicU8,
    /// Reconnect now (a fresh first frame), not after [`RETRY`].
    refresh: AtomicBool,
    stop: AtomicBool,
}

/// The page a preview shows, as the stream last said.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Page {
    pub(crate) title: String,
    pub(crate) url: String,
    /// The page's size in CSS pixels, from the last frame.
    pub(crate) viewport: (f32, f32),
    pub(crate) frames: u64,
    /// Why there's no stream, while there isn't.
    pub(crate) failed: Option<String>,
}

impl Preview {
    fn new(session: String, name: String) -> Self {
        Self {
            session,
            name,
            pixels: rune::Pixels::new(),
            full: rune::Pixels::new(),
            page: Mutex::new(Page::default()),
            writer: Mutex::new(None),
            rate: AtomicU8::new(Rate::Idle as u8),
            refresh: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        }
    }

    pub(crate) fn page(&self) -> Page {
        self.page
            .lock()
            .map(|page| page.clone())
            .unwrap_or_default()
    }

    /// Whether a frame has come: else its card shows the page's words.
    pub(crate) fn has_frame(&self) -> bool {
        self.pixels.size() != (0, 0)
    }

    /// The sharpest picture there is: the full frame once it came, else the
    /// thumbnail (for the moment until it does).
    pub(crate) fn large_pixels(&self) -> &rune::Pixels {
        if self.full.size() != (0, 0) {
            &self.full
        } else {
            &self.pixels
        }
    }

    fn rate(&self) -> Rate {
        Rate::from_u8(self.rate.load(Ordering::Acquire))
    }

    fn set_rate(&self, rate: Rate) {
        let before = Rate::from_u8(self.rate.swap(rate as u8, Ordering::AcqRel));
        if before == rate {
            return;
        }
        if rate == Rate::Idle {
            // Nothing draws it: drop the stream until something does.
            if let Ok(writer) = self.writer.lock()
                && let Some(writer) = writer.as_ref()
            {
                writer.close();
            }
            return;
        }
        self.write(&stream::max_fps(rate.fps()));
        if needs_refresh(before, rate, self.full.size() != (0, 0)) {
            // A still page sends no new frame: reconnect for a fresh one.
            self.refresh.store(true, Ordering::Release);
            if let Ok(writer) = self.writer.lock()
                && let Some(writer) = writer.as_ref()
            {
                writer.close();
            }
        }
    }

    fn write(&self, text: &str) {
        if let Ok(writer) = self.writer.lock()
            && let Some(writer) = writer.as_ref()
        {
            let _ = writer.send(text);
        }
    }

    fn halt(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut writer) = self.writer.lock()
            && let Some(writer) = writer.take()
        {
            writer.close();
        }
    }
}

/// A watched task's browsers.
struct Watch {
    previews: Vec<Arc<Preview>>,
    stop: Arc<AtomicBool>,
    /// Painted by a fixture ([`fake`]): no browser behind it, so it is never
    /// listed, and it stays until a fixture replaces it, whatever shows.
    fixture: bool,
}

/// Who shows a browser large.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Viewer {
    /// The overlay's enlarged card.
    Overlay,
    /// The live sheet over Chats.
    Chats,
}

#[derive(Default)]
struct Registry {
    /// The tasks that work now: their browsers stream in the background.
    tracked: Vec<String>,
    /// The thread listing browsers runs.
    listing: bool,
    hovered: Option<String>,
    /// The jobs whose cards show at the orb: their first browsers stream.
    cards: Vec<String>,
    /// The running jobs the Assistant window shows: every browser of
    /// theirs streams slowly while it's in front, so its job cards show
    /// them live whether or not the orb's cards are open.
    window: Vec<String>,
    /// The tasks on the live wall (a job and its helpers): every browser
    /// of theirs streams at [`Rate::Wall`] while it shows.
    wall: Vec<String>,
    /// What each viewer shows large: (task, session).
    viewing: HashMap<Viewer, (String, String)>,
    /// A preview pressed in the orb's list, for Chats to open.
    requested: Option<(String, String)>,
    tasks: HashMap<String, Watch>,
}

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

type Hook = Box<dyn Fn() + Send + Sync>;

fn hooks() -> std::sync::MutexGuard<'static, Vec<Hook>> {
    static HOOKS: OnceLock<Mutex<Vec<Hook>>> = OnceLock::new();
    HOOKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Calls `hook` (on any thread) whenever a browser comes or goes, a page
/// changes, or a preview is requested.
pub(crate) fn on_change(hook: impl Fn() + Send + Sync + 'static) {
    hooks().push(Box::new(hook));
}

fn changed() {
    for hook in hooks().iter() {
        hook();
    }
}

/// Calls the [`on_change`] hooks: an app window's picture came or went
/// (`app_live`).
pub(super) fn notify() {
    changed();
}

/// The tasks that work now (queued, running, paused, waiting): their
/// browsers stream in the background from the moment they open, so a
/// preview shows at once. A task no longer listed stops streaming.
pub(crate) fn track(tasks: &[String]) {
    let mut registry = registry();
    let mut tasks = tasks.to_vec();
    tasks.sort();
    tasks.dedup();
    if registry.tracked == tasks {
        return;
    }
    registry.tracked = tasks;
    sync(&mut registry);
}

/// The jobs whose cards show at the orb now: the first
/// [`super::jobs::MAX_THUMBS`] browsers of each stream slowly; every other
/// browser not hovered or viewed large streams nothing.
pub(crate) fn cards(tasks: &[String]) {
    let mut registry = registry();
    let mut tasks = tasks.to_vec();
    tasks.sort();
    tasks.dedup();
    if registry.cards == tasks {
        return;
    }
    registry.cards = tasks;
    sync(&mut registry);
}

/// The tasks the live wall shows (empty: it's closed): all their browsers
/// stream at [`Rate::Wall`], and stop again when it closes.
pub(crate) fn wall(tasks: &[String]) {
    let mut registry = registry();
    if registry.wall == tasks {
        return;
    }
    registry.wall = tasks.to_vec();
    sync(&mut registry);
}

/// The running jobs the Assistant window shows (empty: it's hidden or on
/// another pane): all their browsers stream slowly for its job cards.
pub(crate) fn window(tasks: &[String]) {
    let mut registry = registry();
    let mut tasks = tasks.to_vec();
    tasks.sort();
    tasks.dedup();
    if registry.window == tasks {
        return;
    }
    registry.window = tasks;
    sync(&mut registry);
}

/// The pointer rests on `task` in the orb's list (`None`: on none).
pub(crate) fn hover(task: Option<&str>) {
    let mut registry = registry();
    if registry.hovered.as_deref() == task {
        return;
    }
    registry.hovered = task.map(str::to_owned);
    sync(&mut registry);
}

/// `viewer` shows `session` of `task` large (`None`: nothing).
pub(crate) fn view(viewer: Viewer, target: Option<(&str, &str)>) {
    let mut registry = registry();
    let target = target.map(|(task, session)| (task.to_owned(), session.to_owned()));
    if registry.viewing.get(&viewer) == target.as_ref() {
        return;
    }
    match target {
        Some(target) => registry.viewing.insert(viewer, target),
        None => registry.viewing.remove(&viewer),
    };
    sync(&mut registry);
}

/// A preview pressed in the orb's list: Chats opens it large.
pub(crate) fn request_view(task: &str, session: &str) {
    registry().requested = Some((task.to_owned(), session.to_owned()));
    changed();
}

/// The preview pressed since the last call, if one was.
pub(crate) fn take_request() -> Option<(String, String)> {
    registry().requested.take()
}

/// `task`'s browsers, main one first, while they're watched.
pub(crate) fn previews(task: &str) -> Vec<Arc<Preview>> {
    registry()
        .tasks
        .get(task)
        .map(|watch| watch.previews.clone())
        .unwrap_or_default()
}

/// The playground's stories, renders, and layout tests: `task`'s browsers are `pages` (session, name,
/// title, url), each showing the picture `paint` draws for its index.
pub(crate) fn fake(
    task: &str,
    pages: &[(&str, &str, &str, &str)],
    paint: impl Fn(usize) -> (u32, u32, Vec<u8>),
) {
    let previews = pages
        .iter()
        .enumerate()
        .map(|(index, (session, name, title, url))| {
            let preview = Preview::new((*session).into(), (*name).into());
            let (width, height, rgba) = paint(index);
            preview.full.set(width, height, rgba.clone());
            preview.pixels.set(width, height, rgba);
            if let Ok(mut page) = preview.page.lock() {
                page.title = (*title).into();
                page.url = (*url).into();
                page.viewport = (width as f32, height as f32);
                page.frames = 1;
            }
            Arc::new(preview)
        })
        .collect();
    registry().tasks.insert(
        task.into(),
        Watch {
            previews,
            stop: Arc::new(AtomicBool::new(true)),
            fixture: true,
        },
    );
}

/// One of `task`'s browsers, while it's watched.
pub(crate) fn preview(task: &str, session: &str) -> Option<Arc<Preview>> {
    previews(task)
        .into_iter()
        .find(|preview| preview.session == session)
}

/// Replays the person's `input` in `session` of `task`.
pub(crate) fn send(task: &str, session: &str, input: &Input) {
    if let Some(preview) = preview(task, session) {
        for message in stream::encode(input) {
            preview.write(&message);
        }
    }
}

/// The tasks to watch: the hovered one, those viewed large, and those that
/// work, each once.
fn wanted(registry: &Registry) -> Vec<String> {
    let mut tasks: Vec<String> = registry.hovered.iter().cloned().collect();
    let mut viewed: Vec<&String> = registry.viewing.values().map(|(task, _)| task).collect();
    viewed.sort();
    for task in viewed
        .into_iter()
        .chain(&registry.wall)
        .chain(&registry.window)
        .chain(&registry.tracked)
    {
        if !tasks.contains(task) {
            tasks.push(task.clone());
        }
    }
    tasks
}

/// How fast `session` of `task` streams: large, hovered, on a card that
/// shows (its first few browsers), or not at all.
fn rate_of(registry: &Registry, task: &str, session: &str) -> Rate {
    let on_card = || {
        registry.cards.iter().any(|card| card == task)
            && registry.tasks.get(task).is_none_or(|watch| {
                watch
                    .previews
                    .iter()
                    .position(|preview| preview.session == session)
                    .is_none_or(|at| at < super::jobs::MAX_THUMBS)
            })
    };
    if large(registry, task, session) {
        Rate::Large
    } else if registry.hovered.as_deref() == Some(task) {
        Rate::Small
    } else if registry.wall.iter().any(|walled| walled == task) {
        Rate::Wall
    } else if on_card() || registry.window.iter().any(|shown| shown == task) {
        Rate::Background
    } else {
        Rate::Idle
    }
}

/// Whether going from `before` to `after` needs a fresh frame now: the
/// person looks at it (hovered or large) and it has no full frame yet.
fn needs_refresh(before: Rate, after: Rate, has_full: bool) -> bool {
    after > before && after >= Rate::Wall && !has_full
}

/// Whether any viewer shows `session` of `task` large.
fn large(registry: &Registry, task: &str, session: &str) -> bool {
    registry
        .viewing
        .values()
        .any(|(t, s)| t == task && s == session)
}

/// Starts and stops the tasks' watchers to match what shows.
fn sync(registry: &mut Registry) {
    let wanted = wanted(registry);
    let gone: Vec<String> = registry
        .tasks
        .iter()
        .filter(|(task, watch)| !watch.fixture && !wanted.contains(task))
        .map(|(task, _)| task.clone())
        .collect();
    for task in gone {
        if let Some(watch) = registry.tasks.remove(&task) {
            watch.stop.store(true, Ordering::Release);
            for preview in watch.previews {
                preview.halt();
            }
        }
    }
    for task in &wanted {
        if !registry.tasks.contains_key(task) {
            registry.tasks.insert(
                task.clone(),
                Watch {
                    previews: Vec::new(),
                    stop: Arc::new(AtomicBool::new(false)),
                    fixture: false,
                },
            );
        }
    }
    let rates: Vec<(Arc<Preview>, Rate)> = registry
        .tasks
        .iter()
        .flat_map(|(task, watch)| {
            watch
                .previews
                .iter()
                .map(|preview| (preview.clone(), rate_of(registry, task, &preview.session)))
                .collect::<Vec<_>>()
        })
        .collect();
    for (preview, rate) in rates {
        preview.set_rate(rate);
    }
    if !wanted.is_empty() && !registry.listing {
        registry.listing = true;
        let started = std::thread::Builder::new()
            .name("hex-browser-list".into())
            .spawn(list);
        if started.is_err() {
            registry.listing = false;
        }
    }
}

/// Lists every watched task's browsers every [`RELIST`] (one agent-browser
/// call for all of them), streaming each, until no task is watched.
fn list() {
    let Some(path) = crate::assistant::browser::executable() else {
        registry().listing = false;
        return;
    };
    loop {
        let tasks = {
            let mut registry = registry();
            let tasks: Vec<String> = registry
                .tasks
                .iter()
                .filter(|(_, watch)| !watch.fixture)
                .map(|(task, _)| task.clone())
                .collect();
            if tasks.is_empty() {
                registry.listing = false;
                return;
            }
            tasks
        };
        let mut open = crate::assistant::browser::running_of(&path, &tasks);
        let mut any = false;
        for task in &tasks {
            let mut sessions = open.remove(task).unwrap_or_default();
            sessions.sort_by_key(|session| (session.contains('_'), session.clone()));
            any |= apply_sessions(task, &sessions, &path);
        }
        if any {
            changed();
        }
        std::thread::sleep(RELIST);
    }
}

/// Matches `task`'s previews to its open `sessions`, starting streams for
/// new ones; whether anything changed.
fn apply_sessions(task: &str, sessions: &[String], path: &std::path::Path) -> bool {
    let mut registry = registry();
    let rates: Vec<Rate> = sessions
        .iter()
        .map(|session| rate_of(&registry, task, session))
        .collect();
    let Some(watch) = registry.tasks.get_mut(task) else {
        return false;
    };
    if watch.stop.load(Ordering::Acquire) {
        return false;
    }
    let before = watch.previews.len();
    watch.previews.retain(|preview| {
        let open = sessions.contains(&preview.session);
        if !open {
            preview.halt();
        }
        open
    });
    let mut changed = watch.previews.len() != before;
    for (session, rate) in sessions.iter().zip(rates) {
        if watch
            .previews
            .iter()
            .any(|preview| &preview.session == session)
        {
            continue;
        }
        let preview = Arc::new(Preview::new(session.clone(), name_of(task, session)));
        preview.rate.store(rate as u8, Ordering::Release);
        watch.previews.push(preview.clone());
        let path = path.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("hex-browser-stream".into())
            .spawn(move || run(&preview, &path));
        changed = true;
    }
    // A browser's place on its card may have moved (one before it closed):
    // the first few stream, the rest go idle.
    let previews = watch.previews.clone();
    for preview in previews {
        preview.set_rate(rate_of(&registry, task, &preview.session));
    }
    changed
}

/// What the agent calls `session` of `task`: "" for its main browser.
fn name_of(task: &str, session: &str) -> String {
    let main = crate::assistant::browser::session(task);
    session
        .strip_prefix(&main)
        .map(|rest| rest.trim_start_matches('_').to_owned())
        .unwrap_or_else(|| session.to_owned())
}

/// The host of `url` without `www.` ("example.com"), for a caption.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.strip_prefix("www.").unwrap_or(host).to_owned())
}

#[cfg(test)]
mod tests {
    use super::frames::decode;
    use super::*;

    #[test]
    fn a_browser_is_named_as_the_agent_names_it() {
        let main = crate::assistant::browser::session("t1");
        assert_eq!(name_of("t1", &main), "");
        assert_eq!(name_of("t1", &format!("{main}_suppliers-2")), "suppliers-2");
    }

    #[test]
    fn the_hovered_and_the_viewed_task_are_watched() {
        let mut registry = Registry {
            hovered: Some("a".into()),
            ..Registry::default()
        };
        registry
            .viewing
            .insert(Viewer::Overlay, ("b".into(), "hex-b".into()));
        assert_eq!(wanted(&registry), ["a", "b"]);
        registry
            .viewing
            .insert(Viewer::Overlay, ("a".into(), "hex-a".into()));
        assert_eq!(wanted(&registry), ["a"]);
        registry.hovered = None;
        registry.viewing.clear();
        assert!(wanted(&registry).is_empty());
    }

    #[test]
    fn two_viewers_of_one_browser_share_its_stream() {
        let mut registry = Registry::default();
        registry
            .viewing
            .insert(Viewer::Overlay, ("t".into(), "hex-t".into()));
        registry
            .viewing
            .insert(Viewer::Chats, ("t".into(), "hex-t".into()));
        // One task watched, so one stream per browser of it.
        assert_eq!(wanted(&registry), ["t"]);
        assert!(large(&registry, "t", "hex-t"));
        assert!(!large(&registry, "t", "hex-t_b"));
        // Closing one viewer keeps it large for the other.
        registry.viewing.remove(&Viewer::Overlay);
        assert!(large(&registry, "t", "hex-t"));
        registry.viewing.remove(&Viewer::Chats);
        assert!(!large(&registry, "t", "hex-t"));
        assert!(wanted(&registry).is_empty());
    }

    #[test]
    fn a_thumbnail_is_filtered_to_its_width_and_never_upscaled() {
        // 4×2: a black and a white 2×2 block.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            rgba.extend([0, 0, 0, 255].repeat(2));
            rgba.extend([255, 255, 255, 255].repeat(2));
        }
        let (w, h, half) = fit_width(&rgba, 4, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(half, [0, 0, 0, 255, 255, 255, 255, 255]);
        // Not a power of two: 1920 wide lands at the asked width, not 240.
        let wide = vec![128u8; 1920 * 1200 * 4];
        let (w, h, out) = fit_width(&wide, 1920, 1200, THUMB_WIDTH);
        assert_eq!((w, h), (THUMB_WIDTH, 225));
        assert!(out.iter().all(|value| *value == 128));
        // Narrower than asked: kept as it is.
        assert_eq!(fit_width(&rgba, 4, 2, 360).0, 4);
    }

    #[test]
    fn a_frame_decodes_at_its_own_size() {
        let jpeg = include_bytes!("../../../tests/fixtures/live-frame.jpg");
        let (width, height, rgba) = decode(jpeg).unwrap();
        assert_eq!((width, height), (64, 32));
        assert_eq!(rgba.len(), 64 * 32 * 4);
        assert!(decode(b"not a jpeg").is_none());
    }

    #[test]
    fn only_what_shows_streams() {
        let preview = |session: &str| Arc::new(Preview::new(session.into(), String::new()));
        let mut registry = Registry {
            tracked: vec!["carded".into(), "folded".into()],
            cards: vec!["carded".into()],
            ..Registry::default()
        };
        let sessions: Vec<String> = (0..6).map(|n| format!("hex-carded_{n}")).collect();
        registry.tasks.insert(
            "carded".into(),
            Watch {
                previews: sessions.iter().map(|s| preview(s)).collect(),
                stop: Arc::new(AtomicBool::new(false)),
                fixture: false,
            },
        );
        // A card's first three browsers stream slowly; the fourth on is "+3".
        assert_eq!(
            rate_of(&registry, "carded", "hex-carded_0"),
            Rate::Background
        );
        assert_eq!(
            rate_of(&registry, "carded", "hex-carded_2"),
            Rate::Background
        );
        assert_eq!(rate_of(&registry, "carded", "hex-carded_3"), Rate::Idle);
        // A job folded into the count line streams nothing until it shows.
        assert_eq!(rate_of(&registry, "folded", "hex-folded"), Rate::Idle);
        registry.hovered = Some("folded".into());
        assert_eq!(rate_of(&registry, "folded", "hex-folded"), Rate::Small);
        registry
            .viewing
            .insert(Viewer::Chats, ("carded".into(), "hex-carded_5".into()));
        assert_eq!(rate_of(&registry, "carded", "hex-carded_5"), Rate::Large);
        assert!(Rate::Idle < Rate::Background);
        assert_eq!(Rate::from_u8(Rate::Idle as u8), Rate::Idle);
        assert_eq!(Rate::from_u8(Rate::Large as u8), Rate::Large);
    }

    #[test]
    fn a_fixture_s_browsers_stay_whatever_shows_and_a_task_s_go() {
        let watch = |fixture| Watch {
            previews: vec![Arc::new(Preview::new("hex-x".into(), String::new()))],
            stop: Arc::new(AtomicBool::new(fixture)),
            fixture,
        };
        let mut registry = Registry::default();
        registry.tasks.insert("painted".into(), watch(true));
        registry.tasks.insert("ended".into(), watch(false));
        // Nothing shows (a window hidden, the wall closed): no list starts.
        sync(&mut registry);
        assert!(registry.tasks.contains_key("painted"));
        assert!(!registry.tasks.contains_key("ended"));
        assert!(!registry.listing);
        assert_eq!(registry.tasks["painted"].previews.len(), 1);
    }

    #[test]
    fn working_tasks_stream_in_the_background_and_speed_up_when_looked_at() {
        let mut registry = Registry {
            tracked: vec!["bg".into(), "a".into()],
            cards: vec!["bg".into(), "a".into()],
            hovered: Some("a".into()),
            ..Registry::default()
        };
        registry
            .viewing
            .insert(Viewer::Overlay, ("a".into(), "hex-a_b".into()));
        assert_eq!(wanted(&registry), ["a", "bg"]);
        assert_eq!(rate_of(&registry, "bg", "hex-bg"), Rate::Background);
        assert_eq!(rate_of(&registry, "a", "hex-a"), Rate::Small);
        assert_eq!(rate_of(&registry, "a", "hex-a_b"), Rate::Large);
        assert!(Rate::Large.fps() > Rate::Small.fps());
        assert!(Rate::Small.fps() > Rate::Background.fps());
    }

    #[test]
    fn the_assistant_window_streams_its_running_jobs_without_the_orb_s_cards() {
        let mut registry = Registry {
            tracked: vec!["job".into(), "other".into()],
            ..Registry::default()
        };
        // No card at the orb and the window hidden: nothing streams.
        assert_eq!(rate_of(&registry, "job", "hex-job"), Rate::Idle);
        registry.window = vec!["job".into()];
        assert_eq!(rate_of(&registry, "job", "hex-job"), Rate::Background);
        assert_eq!(
            rate_of(&registry, "job", "hex-job_content"),
            Rate::Background
        );
        assert_eq!(rate_of(&registry, "other", "hex-other"), Rate::Idle);
        assert!(wanted(&registry).contains(&"job".to_owned()));
    }

    #[test]
    fn the_wall_streams_every_browser_of_its_jobs_only_while_it_shows() {
        let preview = |session: &str| Arc::new(Preview::new(session.into(), String::new()));
        let mut registry = Registry {
            tracked: vec!["lead".into(), "h1".into(), "other".into()],
            cards: vec!["lead".into()],
            ..Registry::default()
        };
        let sessions: Vec<String> = (0..20).map(|n| format!("hex-h1_{n}")).collect();
        registry.tasks.insert(
            "h1".into(),
            Watch {
                previews: sessions.iter().map(|s| preview(s)).collect(),
                stop: Arc::new(AtomicBool::new(false)),
                fixture: false,
            },
        );
        // Closed: a helper's browsers past a card stream nothing.
        assert_eq!(rate_of(&registry, "h1", "hex-h1_19"), Rate::Idle);
        registry.wall = vec!["lead".into(), "h1".into()];
        for session in &sessions {
            assert_eq!(rate_of(&registry, "h1", session), Rate::Wall);
        }
        assert_eq!(rate_of(&registry, "lead", "hex-lead"), Rate::Wall);
        assert_eq!(rate_of(&registry, "other", "hex-other"), Rate::Idle);
        // Pressed large or hovered, one goes faster.
        registry
            .viewing
            .insert(Viewer::Overlay, ("h1".into(), "hex-h1_3".into()));
        assert_eq!(rate_of(&registry, "h1", "hex-h1_3"), Rate::Large);
        // One or two frames a second: sharp but cheap.
        assert!((1..=2).contains(&Rate::Wall.fps()));
        assert!(Rate::Background < Rate::Wall && Rate::Wall < Rate::Small);
        assert!(needs_refresh(Rate::Idle, Rate::Wall, false));
        // Closed again: back to what the cards show.
        registry.wall.clear();
        assert_eq!(rate_of(&registry, "h1", "hex-h1_19"), Rate::Idle);
        assert_eq!(Rate::from_u8(Rate::Wall as u8), Rate::Wall);
    }

    #[test]
    fn looking_closer_without_a_full_frame_asks_for_a_fresh_one() {
        assert!(needs_refresh(Rate::Background, Rate::Small, false));
        assert!(needs_refresh(Rate::Small, Rate::Large, false));
        assert!(
            !needs_refresh(Rate::Background, Rate::Large, true),
            "has one"
        );
        assert!(
            !needs_refresh(Rate::Large, Rate::Small, false),
            "slowing down"
        );
        assert!(!needs_refresh(Rate::Small, Rate::Background, false));
    }

    #[test]
    fn captions_name_the_site_and_keys_reach_the_page() {
        assert_eq!(
            host_of("https://www.example.com/a?b").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("about:blank"), None);
        let mods = rune::Mods::default();
        assert_eq!(
            key_input(&rune::Key::Text("ä".into()), mods),
            Some(Input::Key {
                key: "ä".into(),
                text: Some("ä".into())
            })
        );
        assert_eq!(
            key_input(&rune::Key::Left, mods),
            Some(Input::Key {
                key: "ArrowLeft".into(),
                text: None
            })
        );
        assert_eq!(key_input(&rune::Key::Escape, mods), None);
        let cmd = rune::Mods {
            cmd: true,
            ..rune::Mods::default()
        };
        assert_eq!(key_input(&rune::Key::Text("w".into()), cmd), None);
    }

    #[test]
    fn a_point_on_the_preview_lands_on_the_page() {
        assert_eq!(
            to_page((320.0, 180.0), (640.0, 360.0), (1280.0, 720.0)),
            (640.0, 360.0)
        );
        assert_eq!(
            to_page((-5.0, 400.0), (640.0, 360.0), (1280.0, 720.0)),
            (0.0, 720.0)
        );
        assert_eq!(to_page((1.0, 1.0), (0.0, 0.0), (1280.0, 720.0)), (0.0, 0.0));
    }
}
