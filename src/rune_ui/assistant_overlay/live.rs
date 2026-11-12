//! Live previews of a task's browsers: while the pointer rests on a task in
//! the orb's list, each of its agent-browser sessions streams its viewport
//! into a small picture ([`hover`]); pressing one opens it large right in
//! the overlay, and "Open in Chats" there opens it large in Chats
//! ([`request_view`], [`view`]); in either, Take over lets the person
//! drive it ([`send`]).
//!
//! Both large views share one stream per browser: [`view`] says which
//! viewer shows what, and a browser any viewer shows large streams large.
//!
//! Streams run only while something shows them: the hovered task's at a
//! few frames a second and downscaled, the one viewed large at full size
//! and faster. Each task has a thread that looks for its browsers every
//! moment and one thread per browser reading its stream
//! (`assistant::browser::stream`). A light Lightpanda browser draws no
//! pixels; its card shows the page's title and address instead.
//!
//! Views read [`previews`] and [`preview`]; frames wake whatever draws
//! their `rune::Pixels`, and a browser coming or going, or its page
//! changing, calls the hooks set with [`on_change`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::assistant::browser::stream::{self, Input, Message, Writer};

/// Frames a second while a preview is small, and while it's viewed large.
const SMALL_FPS: u32 = 3;
const LARGE_FPS: u32 = 15;
/// A small preview's widest, in pixels (two thumbnails side by side at 2×).
const SMALL_WIDTH: usize = 400;
/// How often a watched task's browsers are listed again.
const RELIST: Duration = Duration::from_millis(1500);
/// How long a stream that failed waits before trying again.
const RETRY: Duration = Duration::from_secs(2);

/// One browser's live picture and what its page is.
pub(crate) struct Preview {
    /// The agent-browser session.
    pub(crate) session: String,
    /// What the agent calls it: "" for the task's main browser, else its
    /// name ("a", "suppliers-2").
    pub(crate) name: String,
    pub(crate) pixels: rune::Pixels,
    page: Mutex<Page>,
    writer: Mutex<Option<Writer>>,
    /// Viewed large: full size, more frames.
    large: AtomicBool,
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
            page: Mutex::new(Page::default()),
            writer: Mutex::new(None),
            large: AtomicBool::new(false),
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

    fn set_large(&self, large: bool) {
        if self.large.swap(large, Ordering::AcqRel) != large {
            let fps = if large { LARGE_FPS } else { SMALL_FPS };
            self.write(&stream::max_fps(fps));
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
    hovered: Option<String>,
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

/// The tasks to watch: the hovered one and those viewed large, each once.
fn wanted(registry: &Registry) -> Vec<String> {
    let mut tasks: Vec<String> = registry.hovered.iter().cloned().collect();
    let mut viewed: Vec<&String> = registry.viewing.values().map(|(task, _)| task).collect();
    viewed.sort();
    for task in viewed {
        if !tasks.contains(task) {
            tasks.push(task.clone());
        }
    }
    tasks
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
        .keys()
        .filter(|task| !wanted.contains(task))
        .cloned()
        .collect();
    for task in gone {
        if let Some(watch) = registry.tasks.remove(&task) {
            watch.stop.store(true, Ordering::Release);
            for preview in watch.previews {
                preview.halt();
            }
        }
    }
    for task in wanted {
        if !registry.tasks.contains_key(&task) {
            let stop = Arc::new(AtomicBool::new(false));
            registry.tasks.insert(
                task.clone(),
                Watch {
                    previews: Vec::new(),
                    stop: stop.clone(),
                },
            );
            let _ = std::thread::Builder::new()
                .name("hex-browser-watch".into())
                .spawn(move || watch(&task, &stop));
        }
    }
    for (task, watch) in &registry.tasks {
        for preview in &watch.previews {
            preview.set_large(large(registry, task, &preview.session));
        }
    }
}

/// Lists `task`'s browsers every [`RELIST`] until stopped, streaming each.
fn watch(task: &str, stop: &AtomicBool) {
    let Some(path) = crate::assistant::browser::executable() else {
        return;
    };
    while !stop.load(Ordering::Acquire) {
        let mut sessions = crate::assistant::browser::running(&path, task);
        sessions.sort_by_key(|session| (session.contains('_'), session.clone()));
        if apply_sessions(task, &sessions, &path) {
            changed();
        }
        std::thread::sleep(RELIST);
    }
}

/// Matches `task`'s previews to its open `sessions`, starting streams for
/// new ones; whether anything changed.
fn apply_sessions(task: &str, sessions: &[String], path: &std::path::Path) -> bool {
    let mut registry = registry();
    let viewed: Vec<String> = sessions
        .iter()
        .filter(|session| large(&registry, task, session))
        .cloned()
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
    for session in sessions {
        if watch
            .previews
            .iter()
            .any(|preview| &preview.session == session)
        {
            continue;
        }
        let preview = Arc::new(Preview::new(session.clone(), name_of(task, session)));
        preview
            .large
            .store(viewed.contains(session), Ordering::Release);
        watch.previews.push(preview.clone());
        let path = path.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("hex-browser-stream".into())
            .spawn(move || run(&preview, &path));
        changed = true;
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

/// Streams `preview` until it's halted, reconnecting after a failure.
fn run(preview: &Preview, path: &std::path::Path) {
    while !preview.stop.load(Ordering::Acquire) {
        if let Err(why) = stream_once(preview, path) {
            set_page(preview, |page| page.failed = Some(why.clone()));
            tracing::debug!(session = %preview.session, %why, "browser preview stream ended");
            std::thread::sleep(RETRY);
        }
    }
}

fn stream_once(preview: &Preview, path: &std::path::Path) -> Result<(), String> {
    let port = stream::enable(path, &preview.session)?;
    let fps = if preview.large.load(Ordering::Acquire) {
        LARGE_FPS
    } else {
        SMALL_FPS
    };
    let (mut reader, writer) = stream::connect(port, &format!("pacing=ack&maxFps={fps}"))
        .map_err(|error| error.to_string())?;
    if let Ok(mut slot) = preview.writer.lock() {
        *slot = Some(writer.clone());
    }
    set_page(preview, |page| page.failed = None);
    let seen = AtomicU64::new(0);
    while !preview.stop.load(Ordering::Acquire) {
        let Some(text) = reader.next().map_err(|error| error.to_string())? else {
            return Err("the stream closed".into());
        };
        match stream::parse(&text) {
            Some(Message::Frame {
                seq,
                jpeg,
                viewport,
            }) => {
                let large = preview.large.load(Ordering::Acquire);
                if let Some((width, height, rgba)) =
                    decode(&jpeg, if large { usize::MAX } else { SMALL_WIDTH })
                {
                    preview.pixels.set(width, height, rgba);
                }
                let _ = writer.send(&stream::ack(seq));
                let first = seen.fetch_add(1, Ordering::AcqRel) == 0;
                set_page(preview, |page| {
                    page.viewport = viewport;
                    page.frames += 1;
                });
                if first {
                    changed();
                }
            }
            Some(Message::Url(url)) => {
                if set_page(preview, |page| page.url = url) {
                    changed();
                }
            }
            Some(Message::Tabs { title, url }) => {
                if set_page(preview, |page| {
                    page.title = title;
                    if !url.is_empty() {
                        page.url = url;
                    }
                }) {
                    changed();
                }
            }
            Some(Message::Other) | None => {}
        }
    }
    Ok(())
}

/// Changes `preview`'s page; whether its words changed.
fn set_page(preview: &Preview, change: impl FnOnce(&mut Page)) -> bool {
    let Ok(mut page) = preview.page.lock() else {
        return false;
    };
    let before = (page.title.clone(), page.url.clone(), page.failed.clone());
    change(&mut page);
    before != (page.title.clone(), page.url.clone(), page.failed.clone())
}

/// A JPEG frame as RGBA, at most `max_width` wide (halved until it fits).
fn decode(jpeg: &[u8], max_width: usize) -> Option<(u32, u32, Vec<u8>)> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(jpeg), options);
    let mut rgba = decoder.decode().ok()?;
    let (mut width, mut height) = decoder.dimensions()?;
    if rgba.len() != width * height * 4 {
        return None;
    }
    while width > max_width && width >= 2 && height >= 2 {
        (rgba, width, height) = halve(&rgba, width, height);
    }
    Some((
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        rgba,
    ))
}

/// `rgba` at half the size, each pixel the average of the four it covers.
fn halve(rgba: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (width / 2, height / 2);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let at = |dx: usize, dy: usize| {
                    u32::from(rgba[((y * 2 + dy) * width + x * 2 + dx) * 4 + c])
                };
                out[(y * w + x) * 4 + c] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8;
            }
        }
    }
    (out, w, h)
}

/// The host of `url` without `www.` ("example.com"), for a caption.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.strip_prefix("www.").unwrap_or(host).to_owned())
}

/// The page's key for a key the person pressed on the large view, while
/// they drive it; `None` for what the window keeps (⌘ shortcuts, Escape).
pub(crate) fn key_input(key: &rune::Key, mods: rune::Mods) -> Option<Input> {
    use rune::Key;
    if mods.cmd {
        return None;
    }
    let named = |name: &str, text: Option<&str>| {
        Some(Input::Key {
            key: name.to_owned(),
            text: text.map(str::to_owned),
        })
    };
    match key {
        Key::Text(text) if !text.is_empty() => named(text, Some(text)),
        Key::Enter => named("Enter", Some("\r")),
        Key::Backspace => named("Backspace", None),
        Key::Delete => named("Delete", None),
        Key::Tab => named("Tab", None),
        Key::Left => named("ArrowLeft", None),
        Key::Right => named("ArrowRight", None),
        Key::Up => named("ArrowUp", None),
        Key::Down => named("ArrowDown", None),
        Key::Home => named("Home", None),
        Key::End => named("End", None),
        Key::PageUp => named("PageUp", None),
        Key::PageDown => named("PageDown", None),
        _ => None,
    }
}

/// Where a point on a preview drawn `drawn` big lands on the page, in its
/// CSS pixels.
pub(crate) fn to_page(point: (f32, f32), drawn: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    if drawn.0 <= 0.0 || drawn.1 <= 0.0 {
        return (0.0, 0.0);
    }
    (
        (point.0 / drawn.0 * viewport.0).clamp(0.0, viewport.0),
        (point.1 / drawn.1 * viewport.1).clamp(0.0, viewport.1),
    )
}

#[cfg(test)]
mod tests {
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
    fn a_small_preview_is_halved_until_it_fits() {
        // 4×2: a black and a white 2×2 block.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            rgba.extend([0, 0, 0, 255].repeat(2));
            rgba.extend([255, 255, 255, 255].repeat(2));
        }
        let (half, w, h) = halve(&rgba, 4, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(half, [0, 0, 0, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn a_frame_decodes_and_shrinks() {
        let jpeg = include_bytes!("../../../tests/fixtures/live-frame.jpg");
        let (width, height, rgba) = decode(jpeg, usize::MAX).unwrap();
        assert_eq!((width, height), (64, 32));
        assert_eq!(rgba.len(), 64 * 32 * 4);
        let (width, height, _) = decode(jpeg, 20).unwrap();
        assert_eq!((width, height), (16, 8));
        assert!(decode(b"not a jpeg", 100).is_none());
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
