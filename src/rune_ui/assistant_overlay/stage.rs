//! The stage: where long answers and results are projected, under the
//! island at the top centre of the display the person is on, so they never
//! need Agent-Desk's window (docs/ux/redesign.md §5). One panel at a time:
//!
//! - **Answer**: a Here answer in full (Markdown, scrolling, never cut),
//!   with Copy, Save as note, pin, and what it names to open.
//! - **Result**: a job's result: the headline, the rest as a preview, its
//!   folders, files, and links as rows with their own verbs (Open, Show,
//!   Open in Finder, Open link), What it did, and Try again on a failure.
//! - **Steps**: what a job did, step by step (What it did), with Back.
//! - **Repair**: Agent-Desk needs something again (a permission, the sign-in):
//!   a small card with the one fix it can do in place, Later, and Fix all…
//!   (the only way from here into the window's guide).
//!
//! "Open in Agent-Desk" sits behind ⋯ in the header: the window opens only
//! when asked. Voice acts on the panel's targets ("open the folder", "zeig
//! die Datei", [`spoken`]). Plain state and plain rules, so the host
//! (`host/stage.rs`) only places the panel and does what a press asks.
//!
//! The card's anatomy (header · body · footer) is a thin local [`card`]
//! with the shape of the shared `look::card`; switching is one call.

use std::path::Path;
use std::time::{Duration, Instant};

use rune::{Color, Cx, El, Icon, ScrollSpec, Shadow, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant};

use super::super::controls::Paint;
use super::super::icons::{FOLDER, GLOBE, SPARKLES};
use super::palette;
use super::results::{Kind, Outcome, Target};
use crate::assistant::ConversationId;
use crate::desktop::i18n::{tr, tr_fill};
use crate::desktop::setup_gaps::Gap;

/// The panel's width (the stage of §3.1: 560–640).
pub(crate) const WIDTH: f32 = 640.0;
/// A repair card's width (a rail card's).
pub(crate) const NOTICE_WIDTH: f32 = super::look::RAIL_W;
/// Room around the card for its shadow.
pub(crate) const INSET: f32 = 24.0;
/// The overlay: the card and its shadow, at most about 70 % of a laptop
/// display's height.
pub(crate) const SIZE: (f32, f32) = (WIDTH + 2.0 * INSET, 660.0);
/// How far under the top of the display the stage hangs (past the island).
pub(crate) const TOP: f32 = 44.0;
/// A long answer's tallest; it scrolls past it.
const ANSWER_MAX_H: f32 = 400.0;
/// A result's preview's tallest.
const PREVIEW_MAX_H: f32 = 190.0;
/// What it did: the steps' tallest.
const STEPS_MAX_H: f32 = 380.0;
/// How long a panel nobody looks at stays before it folds away (into the
/// rail's Done group, where a press brings it back).
pub(crate) const STAYS: Duration = Duration::from_secs(20);
/// The most characters of a name a primary button shows.
const NAME_CHARS: usize = 22;

macro_rules! glyph {
    ($name:ident, $id:literal, $body:literal) => {
        static $name: Icon = Icon {
            name: $id,
            view: 24.0,
            body: $body,
        };
    };
}

glyph!(
    PIN,
    "stage-pin",
    "<path d=\"M9 4h6l-1 5 3 3H7l3-3zM12 12v8\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
);
glyph!(
    DOC,
    "stage-doc",
    "<path d=\"M7 3h7l4 4v14H7zM14 3v4h4M9.5 12h6M9.5 15.5h6\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
);
glyph!(
    MORE,
    "stage-more",
    "<path d=\"M6 12h.01M12 12h.01M18 12h.01\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.6\" stroke-linecap=\"round\"/>"
);

/// A Here answer, in full.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Answer {
    pub(crate) id: ConversationId,
    /// The question as asked.
    pub(crate) question: String,
    /// The whole answer (Markdown).
    pub(crate) text: String,
}

/// What a job did, step by step.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Steps {
    pub(crate) id: ConversationId,
    pub(crate) title: String,
    pub(crate) colour: u8,
    pub(crate) steps: Vec<String>,
    /// The job's own record of every tool it used, when its folder has one.
    pub(crate) log: Option<String>,
    /// The panel Back returns to.
    pub(crate) back: Option<Box<Panel>>,
}

/// One panel on the stage.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Panel {
    Answer(Answer),
    /// A job's result, with what it did.
    Result {
        outcome: Outcome,
        steps: Vec<String>,
        log: Option<String>,
    },
    Steps(Steps),
    /// What setup misses again.
    Repair(Vec<Gap>),
}

impl Panel {
    /// What it names to open.
    pub(crate) fn targets(&self) -> Vec<Target> {
        match self {
            Self::Answer(answer) => super::results::targets(&answer.text),
            Self::Result { outcome, .. } => outcome.targets(),
            Self::Steps(_) | Self::Repair(_) => Vec::new(),
        }
    }

    /// Whether it takes the keyboard when it opens (a repair card never
    /// does: it came by itself).
    pub(crate) fn takes_keys(&self) -> bool {
        !matches!(self, Self::Repair(_))
    }
}

/// What a target row's buttons do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Verb {
    /// Opens the file in its app.
    Open,
    /// Shows the file in its folder (Finder).
    Show,
    /// Opens the folder.
    OpenFolder,
    /// Opens the address in the browser.
    OpenLink,
}

impl Verb {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Show => "Show",
            Self::OpenFolder if cfg!(target_os = "macos") => "Open in Finder",
            Self::OpenFolder => "Open folder",
            Self::OpenLink => "Open link",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Show => "show",
            Self::OpenFolder => "folder",
            Self::OpenLink => "link",
        }
    }
}

/// The verbs a target's row offers, the main one last.
pub(crate) fn verbs(kind: Kind) -> &'static [Verb] {
    match kind {
        Kind::Folder => &[Verb::OpenFolder],
        Kind::File => &[Verb::Show, Verb::Open],
        Kind::Link => &[Verb::OpenLink],
    }
}

/// The target the panel's primary action opens: the first file, else the
/// first folder, else the first link.
pub(crate) fn main_target(targets: &[Target]) -> Option<usize> {
    [Kind::File, Kind::Folder, Kind::Link]
        .iter()
        .find_map(|kind| targets.iter().position(|target| target.kind == *kind))
}

/// Where a target is, under its name: the folder it's in (`~` for home),
/// or the address.
pub(crate) fn place(target: &Target, home: Option<&Path>) -> String {
    let tilde = |path: &Path| -> String {
        let shown = path.to_string_lossy().into_owned();
        match home.map(|home| home.to_string_lossy().into_owned()) {
            Some(home) if !home.is_empty() && shown.starts_with(&home) => {
                format!("~{}", &shown[home.len()..])
            }
            _ => shown,
        }
    };
    match target.kind {
        Kind::Link => shorten(&target.open, 56),
        Kind::Folder => {
            let parent = Path::new(&target.open).parent().map(tilde);
            match parent {
                Some(parent) if !parent.is_empty() => {
                    format!("{} · {parent}", tr("Folder"))
                }
                _ => tr("Folder").to_owned(),
            }
        }
        Kind::File => Path::new(&target.open)
            .parent()
            .map(tilde)
            .unwrap_or_default(),
    }
}

fn shorten(words: &str, max: usize) -> String {
    if words.chars().count() <= max {
        return words.to_owned();
    }
    let head: String = words.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// What `words` (a short spoken request) asks of the panel's `targets`:
/// "open the folder", "show the file", "open the sheet", "öffne den
/// Ordner", "zeig die Datei", "open it"; `None` for anything else, which
/// goes to the assistant as usual. Only short requests that start with an
/// open or show verb count, so "open Safari and book a table" never does.
pub(crate) fn spoken(words: &str, targets: &[Target]) -> Option<(usize, Verb)> {
    const OPEN: [&str; 7] = ["open", "öffne", "öffnen", "oeffne", "mach", "ouvre", "abre"];
    const SHOW: [&str; 6] = ["show", "reveal", "zeig", "zeige", "zeigen", "montre"];
    const FILLER: [&str; 14] = [
        "the", "a", "die", "der", "das", "den", "dem", "mir", "me", "bitte", "please", "auf",
        "mal", "up",
    ];
    let lower = words.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '.' && c != '-')
        .map(|token| token.trim_matches('.'))
        .filter(|token| !token.is_empty())
        .collect();
    let (first, rest) = tokens.split_first()?;
    let showing = SHOW.contains(first);
    if !showing && !OPEN.contains(first) {
        return None;
    }
    let rest: Vec<&str> = rest
        .iter()
        .copied()
        .filter(|token| !FILLER.contains(token))
        .collect();
    if rest.is_empty() || rest.len() > 4 {
        return None;
    }
    let first_of = |kind: Kind| targets.iter().position(|target| target.kind == kind);
    let has = |words: &[&str]| rest.iter().any(|token| words.contains(token));
    let at = if has(&["folder", "ordner", "finder", "verzeichnis", "dossier"]) {
        first_of(Kind::Folder)
    } else if has(&[
        "link", "page", "seite", "website", "webseite", "site", "browser",
    ]) {
        first_of(Kind::Link)
    } else if has(&[
        "file",
        "datei",
        "sheet",
        "spreadsheet",
        "tabelle",
        "table",
        "pdf",
        "document",
        "dokument",
        "doc",
        "fichier",
    ]) {
        first_of(Kind::File)
    } else if has(&[
        "it", "es", "ihn", "sie", "result", "ergebnis", "that", "das",
    ]) {
        main_target(targets)
    } else {
        // A target by its own name ("open Vergleich.xlsx", "open muster").
        targets.iter().position(|target| {
            let name = target.name.to_lowercase();
            let stem = name.split('.').next().unwrap_or(&name).to_owned();
            rest.iter()
                .any(|token| token.len() >= 3 && (name == *token || stem == *token))
        })
    }?;
    let verb = match targets[at].kind {
        Kind::Folder => Verb::OpenFolder,
        Kind::Link => Verb::OpenLink,
        Kind::File if showing => Verb::Show,
        Kind::File => Verb::Open,
    };
    Some((at, verb))
}

/// A note's file name for `title`: what a folder allows, at most 60
/// characters, `.md`.
pub(crate) fn note_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    let joined = words.join(" ");
    let joined = joined.trim_matches('.');
    let name: String = joined.chars().take(60).collect();
    let name = name.trim();
    if name.is_empty() {
        "Agent-Desk note.md".into()
    } else {
        format!("{name}.md")
    }
}

/// The note's text: the question as its heading, then the answer.
pub(crate) fn note_text(panel: &Panel) -> Option<(String, String)> {
    match panel {
        Panel::Answer(answer) => Some((
            answer.question.clone(),
            format!("# {}\n\n{}\n", answer.question.trim(), answer.text.trim()),
        )),
        Panel::Result { outcome, .. } => Some((
            outcome.title.clone(),
            format!("# {}\n\n{}\n", outcome.title.trim(), outcome.text.trim()),
        )),
        Panel::Steps(_) | Panel::Repair(_) => None,
    }
}

/// When a panel shown at `since` folds away: never while it's pinned,
/// pointed at, or read aloud; otherwise [`STAYS`] after the last of those.
pub(crate) fn folds_at(since: Instant, pinned: bool, held: bool) -> Option<Instant> {
    (!pinned && !held).then_some(since + STAYS)
}

/// A setup gap the repair card can fix without the window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InPlace {
    /// Opens just this pane of System Settings.
    Accessibility,
    ScreenRecording,
    Microphone,
    Automation,
    /// Runs the agent's own sign-in in Terminal.
    SignIn,
}

/// What [`InPlace`] fixes `gap`, if any.
pub(crate) fn in_place(gap: &Gap) -> Option<InPlace> {
    match gap {
        Gap::Accessibility => Some(InPlace::Accessibility),
        Gap::ScreenRecording => Some(InPlace::ScreenRecording),
        Gap::Microphone => Some(InPlace::Microphone),
        Gap::Automation => Some(InPlace::Automation),
        Gap::AgentSignIn => Some(InPlace::SignIn),
        _ => None,
    }
}

/// What the repair card's buttons ask.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepairAsk {
    Fix(InPlace),
    /// The window's guide for all of it (deliberate).
    FixAll,
    /// Put off until Agent-Desk next starts.
    Later,
}

/// What a press asks of the host.
pub(crate) struct Actions {
    /// ×, Esc: the panel goes.
    pub(crate) close: fn(&mut Stage),
    /// A target's verb (what `results::open` takes, and how).
    pub(crate) open: fn(&mut Stage, &str, Verb),
    /// Save as note.
    pub(crate) save: fn(&mut Stage),
    /// Try again on a failed job.
    pub(crate) retry: fn(&mut Stage, &str),
    /// "Open in Agent-Desk" from ⋯: the one way into the window.
    pub(crate) window: fn(&mut Stage, &str),
    pub(crate) repair: fn(&mut Stage, RepairAsk),
}

impl Actions {
    /// Does nothing (renders, tests).
    #[cfg(test)]
    pub(crate) fn inert() -> Self {
        Self {
            close: |stage| stage.panel = None,
            open: |_, _, _| {},
            save: |stage| stage.saved = true,
            retry: |_, _| {},
            window: |_, _| {},
            repair: |_, _| {},
        }
    }
}

/// The stage's state.
pub(crate) struct Stage {
    pub(crate) panel: Option<Panel>,
    pub(crate) pinned: bool,
    /// ⋯ is open.
    pub(crate) menu: bool,
    /// Copy was pressed for this panel.
    pub(crate) copied: bool,
    /// Save as note worked for this panel.
    pub(crate) saved: bool,
    /// The pointer is on it.
    pub(crate) hovered: bool,
    /// The Action shortcut's keys, for "Hold ⌘ to ask more".
    pub(crate) answer_key: String,
    pub(crate) actions: Actions,
}

impl Stage {
    pub(crate) fn new(answer_key: String, actions: Actions) -> Self {
        Self {
            panel: None,
            pinned: false,
            menu: false,
            copied: false,
            saved: false,
            hovered: false,
            answer_key,
            actions,
        }
    }

    /// Shows `panel` in place of what showed (pinning goes with a panel).
    pub(crate) fn show(&mut self, panel: Panel) {
        if self.panel.as_ref() != Some(&panel) {
            self.pinned = false;
            self.copied = false;
            self.saved = false;
        }
        self.menu = false;
        self.panel = Some(panel);
    }

    /// What it did, for the result showing (Back returns to it).
    pub(crate) fn what_it_did(&mut self) {
        let Some(Panel::Result {
            outcome,
            steps,
            log,
        }) = self.panel.clone()
        else {
            return;
        };
        let back = self.panel.take().map(Box::new);
        self.menu = false;
        self.panel = Some(Panel::Steps(Steps {
            id: outcome.id.clone(),
            title: outcome.title.clone(),
            colour: outcome.colour,
            steps,
            log,
            back,
        }));
    }

    /// Back from What it did.
    pub(crate) fn back(&mut self) {
        if let Some(Panel::Steps(steps)) = &self.panel
            && let Some(back) = steps.back.clone()
        {
            self.panel = Some(*back);
        }
    }
}

/// `content` (a column) scrolling past `max_h`, keyed `key`. The column
/// itself scrolls, so its blocks keep their height.
fn scrolling(content: El, key: &str, max_h: f32) -> El {
    let content = super::wheel::scroller::<Stage>(
        content,
        super::wheel::Surface::Stage,
        key,
        super::wheel::Axis::Y,
    );
    content
        .key(key.to_owned())
        .w_full()
        .max_h(max_h)
        .min_h(0.0)
        .shrink()
        .pr(8.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(14.0)
}

// ---------------------------------------------------------------------------
// Tokens (the redesign's four sizes; `look.rs` has the shared ones)

fn ty_title(p: Paint) -> TextStyle {
    TextStyle {
        size: 17.0,
        line_height: 22.0,
        ..p.k.ty.ui_medium
    }
}

fn ty_body(p: Paint) -> TextStyle {
    TextStyle {
        size: 14.0,
        line_height: 20.0,
        ..p.k.ty.ui
    }
}

fn ty_label(p: Paint) -> TextStyle {
    TextStyle {
        size: 12.0,
        line_height: 16.0,
        ..p.k.ty.small_medium
    }
}

fn ty_meta(p: Paint) -> TextStyle {
    TextStyle {
        size: 11.5,
        line_height: 15.0,
        ..p.k.ty.small
    }
}

/// A state chip: glyph and word on a tint, never colour alone.
fn chip(p: Paint, glyph: &'static Icon, words: &str, colour: Color) -> El {
    row()
        .key("stage-chip")
        .h(20.0)
        .px(7.0)
        .gap(4.0)
        .center_y()
        .radius(10.0)
        .fill(colour.alpha(0.14))
        .child(icon(glyph, 11.0).color(colour).a11y_hidden())
        .child(text(words.to_owned()).font(ty_meta(p)).color(colour))
}

fn dot(colour: Color) -> El {
    el().key("stage-dot")
        .size(9.0, 9.0)
        .radius(4.5)
        .fill(colour)
        .a11y_hidden()
}

/// The card's parts, as the shared anatomy has them.
struct Card {
    key: &'static str,
    lead: El,
    title: String,
    chip: Option<El>,
    /// Pin, ⋯, × at the header's right end.
    tools: Vec<El>,
    body: Vec<El>,
    hint: Option<String>,
    /// At most one primary, last.
    actions: Vec<El>,
    width: f32,
}

/// The shared card anatomy (§3.4): header (who · what · chip), body,
/// footer (voice hint left, actions right). Local until `look::card`.
fn card(p: Paint, card: Card) -> El {
    let c = &p.k.c;
    let header = row()
        .key("stage-header")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(card.lead)
        .child(
            text(card.title.clone())
                .key("stage-title")
                .role("heading")
                .truncate()
                .font(ty_label(p))
                .color(p.muted())
                .grow(1.0)
                .min_w(0.0),
        )
        .children(card.chip)
        .children(card.tools);
    let footer = (card.hint.is_some() || !card.actions.is_empty()).then(|| {
        row()
            .key("stage-footer")
            .w_full()
            .mt(2.0)
            .gap(8.0)
            .center_y()
            .child(el().grow(1.0).min_w(0.0).children(card.hint.map(|hint| {
                text(hint)
                    .key("stage-hint")
                    .truncate()
                    .font(ty_meta(p))
                    .color(p.faint())
            })))
            .children(card.actions)
    });
    col()
        .key(card.key)
        .w(card.width)
        .padding(20.0)
        .gap(12.0)
        .radius(16.0)
        .fill(c.bg_base.alpha(1.0))
        .border(p.hair, c.border_base)
        .box_shadow(Shadow {
            x: 0.0,
            y: 10.0,
            blur: 28.0,
            spread: 0.0,
            color: Color::BLACK.alpha(if p.k.dark { 0.45 } else { 0.16 }),
            inset: false,
        })
        .role("dialog")
        .label(card.title)
        .hit()
        .child(header)
        .children(card.body)
        .children(footer)
}

fn secondary(label: &str) -> Button {
    Button::new(label.to_owned())
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
}

fn primary(label: &str) -> Button {
    Button::new(label.to_owned())
        .variant(ButtonVariant::Submit)
        .control_size(ControlSize::Small)
}

fn tool(glyph: &'static Icon, label: &str, on: bool) -> IconButton {
    IconButton::new(glyph)
        .variant(if on {
            IconButtonVariant::Ghost
        } else {
            IconButtonVariant::GhostMuted
        })
        .control_size(ControlSize::Small)
        .label(label.to_owned())
        .help(label.to_owned())
        .expanded(on)
}

/// Pin, ⋯ and × for `id`'s panel.
fn tools(p: Paint, state: &Stage, id: Option<&str>) -> Vec<El> {
    let close = state.actions.close;
    let mut tools = vec![
        tool(
            &PIN,
            if state.pinned { tr("Unpin") } else { tr("Pin") },
            state.pinned,
        )
        .on_press(|s: &mut Stage| s.pinned = !s.pinned)
        .build(p.k)
        .key("stage-pin"),
    ];
    if id.is_some() {
        tools.push(
            tool(&MORE, tr("More"), state.menu)
                .on_press(|s: &mut Stage| s.menu = !s.menu)
                .build(p.k)
                .key("stage-menu-button"),
        );
    }
    tools.push(
        IconButton::new(p.k.glyph.close)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(tr("Close"))
            .on_press(move |s: &mut Stage| close(s))
            .build(p.k)
            .key("stage-close"),
    );
    tools
}

/// Behind ⋯: Open in Agent-Desk, the window on this conversation.
fn menu(p: Paint, state: &Stage, id: Option<&str>) -> Option<El> {
    let id = id?.to_owned();
    if !state.menu {
        return None;
    }
    let window = state.actions.window;
    Some(
        row().key("stage-menu").w_full().justify_end().child(
            Button::new(tr("Open in Agent-Desk"))
                .variant(ButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut Stage| window(s, &id))
                .build(p.k)
                .key("stage-open-window"),
        ),
    )
}

/// One thing to open: its glyph, name and place, and its verbs.
fn target_row(p: Paint, index: usize, target: &Target, actions: &Actions) -> El {
    let c = &p.k.c;
    let glyph = match target.kind {
        Kind::Folder => &FOLDER,
        Kind::File => &DOC,
        Kind::Link => &GLOBE,
    };
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let open = actions.open;
    let buttons: Vec<El> = verbs(target.kind)
        .iter()
        .map(|verb| {
            let (what, verb) = (target.open.clone(), *verb);
            secondary(tr(verb.label()))
                .on_press(move |s: &mut Stage| open(s, &what, verb))
                .build(p.k)
                .key(format!("stage-verb-{index}-{}", verb.key()))
        })
        .collect();
    row()
        .key(format!("stage-target-{index}"))
        .w_full()
        .py(8.0)
        .gap(10.0)
        .center_y()
        .border_sides([p.hair, 0.0, 0.0, 0.0], c.border_weak)
        .role("listitem")
        .label(target.label())
        .child(
            el().size(32.0, 32.0)
                .radius(8.0)
                .center()
                .fill(c.surface_raised_base)
                .child(icon(glyph, 16.0).color(p.muted()).a11y_hidden()),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(
                    text(target.name.clone())
                        .key("stage-target-name")
                        .truncate()
                        .font(ty_label(p))
                        .color(p.text()),
                )
                .child(
                    text(place(target, home.as_deref()))
                        .key("stage-target-place")
                        .truncate()
                        .font(ty_meta(p))
                        .color(p.faint()),
                ),
        )
        .children(buttons)
}

fn target_rows(p: Paint, targets: &[Target], actions: &Actions) -> Option<El> {
    (!targets.is_empty()).then(|| {
        col().key("stage-targets").w_full().children(
            targets
                .iter()
                .enumerate()
                .map(|(index, target)| target_row(p, index, target, actions)),
        )
    })
}

/// The panel's main action: its main target's verb, with its name, and
/// the hint that says it.
fn primary_open(p: Paint, targets: &[Target], actions: &Actions) -> Option<(El, String)> {
    let at = main_target(targets)?;
    let target = &targets[at];
    let verb = *verbs(target.kind).last()?;
    let label = match target.kind {
        Kind::File => tr_fill("Open {}", &shorten(&target.name, NAME_CHARS)),
        _ => tr(verb.label()).to_owned(),
    };
    let hint = match target.kind {
        Kind::Folder => tr("Say “open the folder”"),
        Kind::File => tr("Say “open the file”"),
        Kind::Link => tr("Say “open the link”"),
    }
    .to_owned();
    let open = actions.open;
    let what = target.open.clone();
    let button = primary(&label)
        .on_press(move |s: &mut Stage| open(s, &what, verb))
        .build(p.k)
        .key("stage-primary");
    Some((button, hint))
}

fn copy_button(p: Paint, state: &Stage, words: String) -> El {
    secondary(if state.copied {
        tr("Copied")
    } else {
        tr("Copy")
    })
    .on_press_tx(move |s: &mut Stage, tx: &mut Tx| {
        tx.copy(words.clone());
        s.copied = true;
    })
    .build(p.k)
    .key("stage-copy")
}

fn save_button(p: Paint, state: &Stage) -> El {
    let save = state.actions.save;
    secondary(if state.saved {
        tr("Saved")
    } else {
        tr("Save as note")
    })
    .on_press(move |s: &mut Stage| save(s))
    .build(p.k)
    .key("stage-save")
}

fn hint_with(state: &Stage, first: Option<String>) -> String {
    let mut parts = Vec::new();
    if let Some(first) = first {
        parts.push(first);
    } else {
        parts.push(tr_fill("Hold {} to ask more", &state.answer_key));
    }
    if state.pinned {
        parts.push(tr("Pinned").to_owned());
    } else {
        parts.push(tr("Esc puts it away").to_owned());
    }
    parts.join(" · ")
}

fn answer(p: Paint, state: &Stage, answer: &Answer) -> El {
    let (lead, _) = palette::frame(p.k.dark);
    let targets = super::results::targets(&answer.text);
    let mut actions = vec![
        copy_button(p, state, answer.text.trim().to_owned()),
        save_button(p, state),
    ];
    let mut hint = None;
    if let Some((button, said)) = primary_open(p, &targets, &state.actions) {
        actions.push(button);
        hint = Some(said);
    }
    let body = scrolling(
        super::markdown::view(p, answer.text.trim()),
        "stage-answer",
        ANSWER_MAX_H,
    );
    card(
        p,
        Card {
            key: "stage-card",
            lead: icon(&SPARKLES, 15.0)
                .key("stage-lead")
                .color(lead)
                .a11y_hidden(),
            title: answer.question.clone(),
            chip: Some(chip(p, p.k.glyph.check, tr("Answered"), p.success())),
            tools: tools(p, state, Some(&answer.id)),
            body: menu(p, state, Some(&answer.id))
                .into_iter()
                .chain([body])
                .chain(target_rows(p, &targets, &state.actions))
                .collect(),
            hint: Some(hint_with(state, hint)),
            actions,
            width: WIDTH,
        },
    )
}

/// An answer without its first line (the headline) and the runner's
/// "Files:" line (its folder has its own row).
pub(crate) fn rest_of(text: &str) -> String {
    let mut lines = text
        .trim()
        .lines()
        .skip_while(|line| line.trim().is_empty());
    lines.next();
    lines
        .filter(|line| !line.trim_start().starts_with("Files:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// A result's headline: its first line in full, as a person reads it (the
/// stage has room; the rail's rows shorten it), else the job's title.
pub(crate) fn headline_of(outcome: &Outcome) -> String {
    outcome
        .text
        .lines()
        .map(super::board::plain)
        .find(|line| !line.trim().is_empty())
        .unwrap_or_else(|| outcome.title.clone())
}

fn result(p: Paint, state: &Stage, outcome: &Outcome, steps: &[String]) -> El {
    let targets = outcome.targets();
    let headline = text(headline_of(outcome))
        .key("stage-headline")
        .wrap()
        .max_lines(3)
        .font(ty_title(p))
        .color(if outcome.failed { p.danger() } else { p.text() })
        .w_full();
    let rest = rest_of(&outcome.text);
    let preview = (!rest.is_empty()).then(|| {
        scrolling(
            super::markdown::view(p, &rest),
            "stage-preview",
            PREVIEW_MAX_H,
        )
    });
    let mut actions = Vec::new();
    if !steps.is_empty() {
        actions.push(
            secondary(tr("What it did"))
                .on_press(|s: &mut Stage| s.what_it_did())
                .build(p.k)
                .key("stage-steps"),
        );
    }
    let mut hint = None;
    if outcome.failed {
        let retry = state.actions.retry;
        let id = outcome.id.clone();
        actions.push(
            primary(tr("Try again"))
                .on_press(move |s: &mut Stage| retry(s, &id))
                .build(p.k)
                .key("stage-retry"),
        );
    } else {
        actions.insert(0, copy_button(p, state, outcome.text.trim().to_owned()));
        if let Some((button, said)) = primary_open(p, &targets, &state.actions) {
            actions.push(button);
            hint = Some(said);
        }
    }
    let (glyph, word, colour) = if outcome.failed {
        (p.k.glyph.circle_x, tr("Failed"), p.danger())
    } else {
        (p.k.glyph.check, tr("Done"), p.success())
    };
    card(
        p,
        Card {
            key: "stage-card",
            lead: dot(palette::task(outcome.colour, p.k.dark)),
            title: outcome.title.clone(),
            chip: Some(chip(p, glyph, word, colour)),
            tools: tools(p, state, Some(&outcome.id)),
            body: menu(p, state, Some(&outcome.id))
                .into_iter()
                .chain([headline])
                .chain(preview)
                .chain(target_rows(p, &targets, &state.actions))
                .collect(),
            hint: Some(hint_with(state, hint)),
            actions,
            width: WIDTH,
        },
    )
}

fn steps(p: Paint, state: &Stage, steps: &Steps) -> El {
    let list = col().key("stage-step-list").w_full().gap(6.0).children(
        steps.steps.iter().enumerate().map(|(index, step)| {
            row()
                .key(format!("stage-step-{index}"))
                .w_full()
                .gap(10.0)
                .items_start()
                .child(
                    text(format!("{}", index + 1))
                        .w(22.0)
                        .font(ty_meta(p))
                        .color(p.faint())
                        .pt(2.0),
                )
                .child(
                    text(step.clone())
                        .wrap()
                        .font(ty_body(p))
                        .color(p.text())
                        .grow(1.0)
                        .min_w(0.0),
                )
        }),
    );
    let body = if steps.steps.is_empty() {
        text(tr("No steps were recorded for this task."))
            .key("stage-steps-body")
            .font(ty_body(p))
            .color(p.muted())
    } else {
        scrolling(list, "stage-steps-body", STEPS_MAX_H)
    };
    let log = steps.log.as_ref().map(|log| {
        let target = Target {
            kind: Kind::File,
            open: log.clone(),
            name: Path::new(log)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| log.clone()),
        };
        target_row(p, 0, &target, &state.actions)
    });
    let mut actions = Vec::new();
    if steps.back.is_some() {
        actions.push(
            Button::new(tr("Back"))
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut Stage| s.back())
                .build(p.k)
                .key("stage-back"),
        );
    }
    card(
        p,
        Card {
            key: "stage-card",
            lead: dot(palette::task(steps.colour, p.k.dark)),
            title: steps.title.clone(),
            chip: Some(chip(p, &DOC, tr("What it did"), p.muted())),
            tools: tools(p, state, Some(&steps.id)),
            body: menu(p, state, Some(&steps.id))
                .into_iter()
                .chain([body])
                .chain(log)
                .collect(),
            hint: Some(if state.pinned {
                tr("Pinned").to_owned()
            } else {
                tr("Esc puts it away").to_owned()
            }),
            actions,
            width: WIDTH,
        },
    )
}

fn repair(p: Paint, state: &Stage, gaps: &[Gap]) -> El {
    let fixable = gaps.iter().find_map(in_place);
    let lines =
        col()
            .key("stage-gaps")
            .w_full()
            .gap(4.0)
            .children(gaps.iter().take(3).enumerate().map(|(index, gap)| {
                text(tr(gap.message()).to_owned())
                    .key(format!("stage-gap-{index}"))
                    .wrap()
                    .max_lines(2)
                    .font(ty_body(p))
                    .color(p.text())
                    .w_full()
            }));
    let ask = state.actions.repair;
    let mut actions = vec![
        Button::new(tr("Later"))
            .variant(ButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut Stage| ask(s, RepairAsk::Later))
            .build(p.k)
            .key("stage-later"),
    ];
    let fix_all = |p: Paint, primary_look: bool| {
        Button::new(tr("Fix all…"))
            .variant(if primary_look {
                ButtonVariant::Submit
            } else {
                ButtonVariant::Outline
            })
            .control_size(ControlSize::Small)
            .on_press(move |s: &mut Stage| ask(s, RepairAsk::FixAll))
            .build(p.k)
            .key("stage-fix-all")
    };
    match fixable {
        Some(fix) => {
            if gaps.len() > 1 {
                actions.push(fix_all(p, false));
            }
            actions.push(
                primary(if fix == InPlace::SignIn {
                    tr("Sign in…")
                } else {
                    tr("Fix")
                })
                .on_press(move |s: &mut Stage| ask(s, RepairAsk::Fix(fix)))
                .build(p.k)
                .key("stage-fix"),
            );
        }
        None => actions.push(fix_all(p, true)),
    }
    let close = state.actions.close;
    card(
        p,
        Card {
            key: "stage-card",
            lead: icon(&SPARKLES, 15.0)
                .key("stage-lead")
                .color(p.warning())
                .a11y_hidden(),
            title: tr("Agent-Desk needs your help").to_owned(),
            chip: Some(chip(p, &MORE, tr("Needs you"), p.warning())),
            tools: vec![
                IconButton::new(p.k.glyph.close)
                    .variant(IconButtonVariant::GhostMuted)
                    .control_size(ControlSize::Small)
                    .label(tr("Close"))
                    .on_press(move |s: &mut Stage| close(s))
                    .build(p.k)
                    .key("stage-close"),
            ],
            body: vec![lines],
            hint: None,
            actions,
            width: NOTICE_WIDTH,
        },
    )
}

pub(crate) fn view(state: &Stage, cx: &mut Cx<Stage>) -> El {
    super::wheel::begin(super::wheel::Surface::Stage);
    drawn(Paint::new(cx.system_dark(), cx.env().scale), state)
}

/// The stage's panel at the top of its overlay (the pretend desk draws it
/// too, to look at; its presses are the stage overlay's).
pub(crate) fn drawn(p: Paint, state: &Stage) -> El {
    let shown = state.panel.as_ref().map(|panel| match panel {
        Panel::Answer(a) => answer(p, state, a),
        Panel::Result { outcome, steps, .. } => result(p, state, outcome, steps),
        Panel::Steps(s) => steps(p, state, s),
        Panel::Repair(gaps) => repair(p, state, gaps),
    });
    col()
        .key("stage-root")
        .w_full()
        .h_full()
        .padding(INSET)
        .items_center()
        .justify_start()
        .children(shown.map(|card| {
            card.on_hover_change(|s: &mut Stage, inside, _| s.hovered = inside)
                .transition(
                    rune::Transition::opacity().combined(rune::Transition::offset(0.0, -8.0)),
                )
        }))
}

#[cfg(test)]
#[path = "stage_tests.rs"]
mod tests;
