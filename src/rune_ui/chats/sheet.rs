//! The assistant's onboarding as it draws: the first run's full-window
//! page, type, card, rows, and motion. One page at a time: its heading, a
//! card of live rows, and Continue (once the page works) beside Later. The
//! self-test pages (`checks`), the shortcut page (`shortcut`), and the
//! gesture tutorial (`try_it`) live beside it.

mod browser;
mod checks;
mod desktop;
mod shortcut;
mod try_it;

pub(super) use browser::sign_in_button;

use rune::{Align, El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, RadioItem, Spinner, TextInput, glyphs};

use super::super::controls::{Paint, PushSize, pulsing_dot};
use super::super::icons::{GLOBE, MIC, SPARKLES};
use super::super::onboarding_kit::{LEAD, Motion, arrives, keys, title_font};
use super::setup::{
    self, Page, SetupFlow, chosen, fix_for, fix_label, page_done,
};
use super::{BACKENDS, Ui, backend_name};
use crate::app_settings::AssistantBackend;
use crate::assistant::backends::{CheckState, Fix};
use crate::desktop::i18n::{tr, tr_fill};

const TOP_BAR: f32 = 48.0;
const ACTION_H: f32 = 32.0;
/// The column the card fills, as wide as the first run's.
const COL_W: f32 = 480.0;
const ROW_H: f32 = 56.0;

static PASSED: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_CHECK;
static FAILED: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;
static CHAT: &rune::Icon = &rune_theme_opencode::icons::SPEECH_BUBBLE;
static SCREEN: &rune::Icon = &rune_theme_opencode::icons::MONITOR;
static TOOL: &rune::Icon = &rune_theme_opencode::icons::CUBE;
static STACK: &rune::Icon = &rune_theme_opencode::icons::BULLET_LIST;
static KEY: &rune::Icon = &rune_theme_opencode::icons::LOCK;

/// The flow over the whole window, while it shows.
pub(in crate::rune_ui) fn overlay(ui: &Ui, p: Paint) -> Option<El> {
    let flow = ui.chats.setup.as_ref()?;
    let page = match flow.page {
        Page::Welcome => welcome(ui, p),
        Page::Agent => agent(ui, flow, p),
        Page::Desktop => desktop::desktop(ui, flow, p),
        Page::Browser => browser::browser(ui, flow, p),
        Page::Checks => checks::checks(ui, flow, p),
        Page::Shortcut => shortcut::shortcut(ui, flow, p),
        Page::Try => try_it::try_it(ui, flow, p),
    };
    Some(
        col()
            .key("assistant-setup")
            .role("dialog")
            .label(tr("Set up the assistant"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(p.k.c.bg_deep)
            .on_press(|_: &mut Ui| {})
            .child(top_bar(ui, flow, p))
            .children(flow.from_hold.then(|| {
                row().key("from-hold").w_full().justify_center().child(
                    text(tr(
                        "Finish setting up the assistant first",
                    ))
                    .key("words")
                    .role("status")
                    .font(p.k.ty.small_medium)
                    .color(p.accent()),
                )
            }))
            .child(
                col()
                    .key("middle")
                    .grow(1.0)
                    .shrink()
                    .w_full()
                    .center()
                    .pb(TOP_BAR)
                    .child(page),
            )
            .modal(true, setup::close),
    )
}

/// Back on the left, where the flow is in the middle, Close on the right.
/// The bar moves the window, as the first run's does.
fn top_bar(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let back = (flow.page != Page::Welcome).then(|| {
        Button::new(tr("Back"))
            .variant(ButtonVariant::Ghost)
            .on_press(setup::back)
            .build_push(p.k)
            .key("back")
            .absolute(20.0, 8.0)
            .transition(arrives())
    });
    let quiet = p.muted();
    let close = row()
        .key("close-corner")
        .absolute(0.0, 8.0)
        .w_full()
        .pr(20.0)
        .justify_end()
        .child(
            Button::new(tr("Close"))
                .variant(ButtonVariant::Ghost)
                .label_with(move |font, _| text(tr("Close")).key("label").font(font).color(quiet))
                .on_press(setup::close)
                .build_push(p.k)
                .key("close-setup"),
        );
    super::super::sidebar::title_bar(row())
        .key("top-bar")
        .w_full()
        .h(TOP_BAR)
        .px(20.0)
        .center_y()
        .justify_center()
        .child(progress(ui, flow, p))
        .child(close)
        .children(back)
}

/// One mark per page: the current one long, finished ones green, ones left
/// with Later amber.
fn progress(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let c = &p.k.c;
    let current = Page::ALL
        .iter()
        .position(|page| *page == flow.page)
        .unwrap_or(0);
    let mut marks = row()
        .key("progress")
        .role("progressbar")
        .label(tr("Setup"))
        .a11y_value(tr_fill(
            "Step {}",
            &format!("{} / {}", current + 1, Page::ALL.len()),
        ))
        .gap(6.0)
        .center_y();
    for (index, page) in Page::ALL.iter().enumerate() {
        let here = index == current;
        let fill = if here {
            c.text_base
        } else if flow.later.contains(page) {
            p.warning()
        } else if index < current && page_done(ui, flow, *page) {
            p.success()
        } else {
            c.border_strong
        };
        marks = marks.child(
            el().key(format!("mark{index}"))
                .size(if here { 18.0 } else { 6.0 }, 6.0)
                .radius(3.0)
                .fill(fill)
                .motion(rune::Spring::new(0.3, 0.0))
                .a11y_hidden(),
        );
    }
    marks
}

/// A page: its heading, its card, a note, and its actions, arriving as
/// the first run's pages do.
fn page(
    p: Paint,
    key: &str,
    title: &str,
    lead: El,
    card: Option<El>,
    note: Option<El>,
    actions: El,
) -> El {
    let motion = Motion::default();
    let heading = motion.heading(
        col()
            .key(format!("{key}-head"))
            .gap(8.0)
            .items_center()
            .child(
                text(title.to_owned())
                    .key("title")
                    .role("heading")
                    .level(1)
                    .font(title_font(p))
                    .color(p.text()),
            )
            .child(lead),
    );
    col()
        .key(key.to_owned())
        .gap(22.0)
        .items_center()
        .child(heading)
        .children(
            card.map(|card| motion.staggered(p.k.elevate(card, &p.k.elevation.raised), 1.0, 8.0)),
        )
        .children(note)
        .child(actions)
}

fn lead(p: Paint, copy: &str) -> El {
    text(copy.to_owned())
        .key("lead")
        .font(LEAD)
        .color(p.muted())
        .align(Align::Center)
        .wrap()
        .max_w(COL_W)
}

/// The first run's setup card.
fn card(p: Paint, key: &str) -> El {
    col()
        .key(key.to_owned())
        .w(COL_W)
        .py(4.0)
        .radius(p.k.radius.lg)
        .fill(p.k.c.bg_base)
        .border(p.hair, p.k.c.border_weak)
}

fn separator(p: Paint, key: String) -> El {
    el().key(key)
        .h(p.hair)
        .mx(16.0)
        .fill(p.k.c.border_muted)
        .a11y_hidden()
}

/// `rows` in a card, a hairline between each.
fn rows_card(p: Paint, key: &str, rows: Vec<El>) -> El {
    let mut card = card(p, key);
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            card = card.child(separator(p, format!("sep{index}")));
        }
        card = card.child(row);
    }
    card
}

/// One row of a card: an icon, its title over what it found, a control.
fn row_of(
    p: Paint,
    key: &str,
    glyph: &'static rune::Icon,
    title: &str,
    detail: &str,
    ink: rune::Color,
    control: El,
) -> El {
    row()
        .key(key.to_owned())
        .role("group")
        .label(title.to_owned())
        .description(detail.to_owned())
        .min_h(ROW_H)
        .py(8.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(
            icon(glyph, 16.0)
                .key("icon")
                .color(p.k.c.icon_base)
                .a11y_hidden(),
        )
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .child(
                    text(title.to_owned())
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(p.text())
                        .wrap()
                        .a11y_hidden(),
                )
                .children((!detail.is_empty()).then(|| {
                    text(detail.to_owned())
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(ink)
                        .wrap()
                        .a11y_hidden()
                })),
        )
        .child(control)
}

/// A calm green check and the word for it.
fn done_pill(p: Paint, key: &str, label: &str) -> El {
    row()
        .key(key.to_owned())
        .h(28.0)
        .px(6.0)
        .gap(5.0)
        .center_y()
        .transition(arrives())
        .child(
            icon(p.k.glyph.check_small, 16.0)
                .key("check")
                .color(p.success())
                .a11y_hidden(),
        )
        .child(
            text(label.to_owned())
                .key("label")
                .font(p.k.ty.ui_medium)
                .color(p.success()),
        )
}

/// Waiting on macOS or the person: a breathing dot and the word.
fn waiting_pill(p: Paint, key: &str, label: &str) -> El {
    let k = p.k;
    row()
        .key(key.to_owned())
        .h(28.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .fill(k.c.surface_raised)
        .border(p.hair, k.c.border_weak)
        .transition(arrives())
        .child(
            el().key("dot")
                .size(8.0, 8.0)
                .child(pulsing_dot(p.accent()))
                .a11y_hidden(),
        )
        .child(
            text(label.to_owned())
                .key("label")
                .font(k.ty.small_medium)
                .color(p.muted()),
        )
}

fn spinner(p: Paint, key: &str) -> El {
    el().key(key.to_owned())
        .size(28.0, 28.0)
        .center()
        .transition(arrives())
        .child(Spinner::new().size(16.0).color(p.muted()).build(p.k))
}

/// A row's action button: the first run's push button, Contrast for the
/// one thing to do next.
fn push(p: Paint, key: &str, label: &str, primary: bool, press: impl Fn(&mut Ui) + 'static) -> El {
    Button::new(label.to_owned())
        .variant(if primary {
            ButtonVariant::Contrast
        } else {
            ButtonVariant::Neutral
        })
        .on_press(press)
        .build_push(p.k)
        .key(key.to_owned())
        .transition(arrives())
}

/// Continue →, enabled once the page works; Later beside it moves on and
/// leaves the page unfinished.
fn actions(p: Paint, done: bool, later: bool) -> El {
    let label = tr("Continue");
    let continue_button = Button::new(label)
        .variant(ButtonVariant::Contrast)
        .disabled(!done)
        .label_with(move |font, ink| {
            row()
                .gap(6.0)
                .center_y()
                .child(text(label).key("label").font(font).color(ink))
                .child(icon(&glyphs::ARROW_RIGHT, 15.0).key("arrow").color(ink))
        })
        .on_press(setup::advance)
        .build_push(p.k)
        .key("continue");
    let continue_button = if done {
        continue_button.default_button()
    } else {
        continue_button
    };
    row()
        .key("action")
        .h(ACTION_H)
        .gap(12.0)
        .center()
        .transition(arrives())
        .children(later.then(|| later_button(p)))
        .child(continue_button)
}

fn later_button(p: Paint) -> El {
    let quiet = p.muted();
    Button::new(tr("Later"))
        .variant(ButtonVariant::Ghost)
        .label_with(move |font, _| text(tr("Later")).key("label").font(font).color(quiet))
        .on_press(setup::later)
        .build_push(p.k)
        .key("later")
}

/// The keys of the Action shortcut, as the first run draws a shortcut.
fn action_keys(ui: &Ui, p: Paint, key: &str, down: bool) -> Option<El> {
    let binding = ui.settings.assistant.action_hotkey.as_ref()?;
    let caps = super::super::settings::shortcut_caps(binding, false);
    Some(keys(p, key, &caps, down))
}

/// Stop all's keys, as words: ⌃⌥Esc.
fn stop_all_words(ui: &Ui) -> String {
    ui.settings
        .assistant
        .stop_all_hotkey
        .as_ref()
        .map(|binding| super::super::settings::shortcut_caps(binding, false).join(""))
        .unwrap_or_else(|| tr("Stop all").to_owned())
}

fn welcome(ui: &Ui, p: Paint) -> El {
    let shortcut = ui
        .settings
        .assistant
        .action_hotkey
        .as_ref()
        .map(|binding| super::super::settings::shortcut_caps(binding, false).join(" "))
        .unwrap_or_else(|| tr("the Action shortcut").to_owned());
    let stop_all = stop_all_words(ui);
    let rows = vec![
        row_of(
            p,
            "ask",
            &MIC,
            &tr_fill("Hold {} to ask about the window in front", &shortcut),
            tr("It answers aloud, and can act in that window"),
            p.muted(),
            el().key("none"),
        ),
        row_of(
            p,
            "background",
            &SPARKLES,
            &tr_fill("Tap, then hold {} for a background task", &shortcut),
            tr("Many run at once in windows of their own, while a frame circles the screen"),
            p.muted(),
            el().key("none"),
        ),
        row_of(
            p,
            "tasks",
            STACK,
            &tr_fill("Double-tap {} to see every task", &shortcut),
            &tr_fill("Take one over, or stop them all with {}", &stop_all),
            p.muted(),
            el().key("none"),
        ),
        row_of(
            p,
            "chats",
            CHAT,
            tr("Chats"),
            tr("Every conversation in Hex's window, and typed questions"),
            p.muted(),
            el().key("none"),
        ),
    ];
    let flow = ui.chats.setup.as_ref();
    let (title, copy) = if flow.is_some_and(|flow| flow.merged) {
        (
            tr("Now, your assistant"),
            tr("Hex can also answer questions about what's on screen and use your Mac for you. A few checks, and you're done."),
        )
    } else if flow.is_some_and(|flow| flow.new_in_hex) {
        (
            tr("New in Hex: your assistant"),
            tr("Ask about what's on screen, or let it use your Mac for you. Hex makes sure every piece works first."),
        )
    } else {
        (
            tr("Meet your assistant"),
            tr("Ask anything by voice, or let it use your Mac for you. Hex makes sure every piece works first."),
        )
    };
    page(
        p,
        "welcome",
        title,
        lead(p, copy),
        Some(rows_card(p, "welcome-card", rows)),
        None,
        actions(p, true, false),
    )
}

/// The agents found; the chosen one answers. Each that isn't ready offers
/// its fix.
fn agent(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let chosen_backend = ui.settings.assistant.backend;
    let found = ui.chats.settings.found.as_deref();
    let mut rows = Vec::new();
    for (index, backend) in BACKENDS.iter().enumerate() {
        let entry = found.and_then(|found| found.iter().find(|entry| entry.backend == *backend));
        let description = match entry {
            None => String::new(),
            Some(entry) if entry.ready => entry
                .version
                .as_ref()
                .map(|version| tr_fill("Version {}", version))
                .unwrap_or_else(|| tr("Ready").to_owned()),
            Some(entry) => entry
                .note
                .clone()
                .unwrap_or_else(|| tr("Not ready").to_owned()),
        };
        let pick = *backend;
        let radio = RadioItem::new(backend_name(*backend), *backend == chosen_backend)
            .description(description)
            .on_select_tx(move |ui: &mut Ui, tx: &mut Tx| {
                tx.instant();
                ui.update(|settings| settings.assistant.backend = pick);
            })
            .build(p.k)
            .key("radio")
            .grow(1.0)
            .shrink();
        let control = match entry {
            None if flow.detecting => spinner(p, "looking"),
            None => el().key("none"),
            Some(entry) if entry.ready => done_pill(p, "ready", tr("Ready")),
            Some(entry) => match fix_for(entry) {
                Some(fix @ Fix::Install(_)) if flow.fixing == Some(fix) => row()
                    .key("installing")
                    .gap(6.0)
                    .center_y()
                    .child(
                        text(
                            flow.fix_log
                                .last()
                                .cloned()
                                .unwrap_or_else(|| tr("Installing…").to_owned()),
                        )
                        .key("label")
                        .font(p.k.ty.small)
                        .color(p.muted())
                        .max_w(200.0)
                        .truncate(),
                    )
                    .child(spinner(p, "fixing")),
                Some(fix) if flow.fixing == Some(fix) => spinner(p, "fixing"),
                Some(Fix::AddApiKey) => el().key("none"),
                Some(fix) => push(
                    p,
                    "fix",
                    &fix_label(fix),
                    *backend == chosen_backend,
                    move |ui: &mut Ui| setup::apply(ui, fix),
                ),
                None => el().key("none"),
            },
        };
        rows.push(
            row()
                .key(format!("agent{index}"))
                .role("none")
                .min_h(ROW_H)
                .mx(4.0)
                .px(12.0)
                .py(10.0)
                .gap(12.0)
                .center_y()
                .radius(p.k.radius.md)
                .hover(p.k.c.hover)
                .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| {
                    tx.instant();
                    ui.update(|settings| settings.assistant.backend = pick);
                })
                .child(radio)
                .child(control),
        );
    }
    if chosen_backend == AssistantBackend::ClaudeApi && !chosen(ui).is_some_and(|found| found.ready)
    {
        rows.push(key_row(ui, p));
    }
    // An install that failed shows what it printed last, and the command
    // to run in Terminal instead.
    let note = match (&flow.fix_error, &ui.chats.settings.key_error) {
        (Some(error), _) => Some(desktop::install_failure(
            p,
            error,
            flow.fix_command.as_deref(),
        )),
        (None, Some(error)) => Some(note(p, error)),
        (None, None) => None,
    };
    let lead_copy = if flow.detecting && found.is_none() {
        tr("Looking for the agents on this Mac…")
    } else {
        tr("It runs on this Mac, signed in with your own account. Claude Code is the default.")
    };
    page(
        p,
        "agent",
        tr("Choose who answers"),
        lead(p, lead_copy),
        Some(rows_card(p, "agent-card", rows)),
        note,
        actions(p, setup::page_done(ui, flow, Page::Agent), true),
    )
}

/// The Claude API key, saved to the Keychain.
fn key_row(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings;
    row()
        .key("api-key")
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .child(
            TextInput::new(&state.key)
                .secure()
                .placeholder(tr("Claude API key"))
                .label(tr("Claude API key"))
                .width(None)
                .on_edit(|ui: &mut Ui| &mut ui.chats.settings.key)
                .on_submit(|ui: &mut Ui, _| super::settings::save_key(ui))
                .invalid(state.key_error.is_some())
                .build(p.k)
                .key("api-key-field")
                .grow(1.0)
                .shrink(),
        )
        .child(push(
            p,
            "save-api-key",
            tr("Save"),
            true,
            super::settings::save_key,
        ))
}

fn note(p: Paint, copy: &str) -> El {
    text(copy.to_owned())
        .key("note")
        .role("status")
        .font(p.k.ty.small)
        .color(p.danger())
        .align(Align::Center)
        .wrap()
        .max_w(COL_W)
        .transition(arrives())
}

/// A self-test's mark: an empty ring, a spinner, a green check, a red cross.
fn mark(p: Paint, state: CheckState) -> El {
    let mark = match state {
        CheckState::Pending => el()
            .size(12.0, 12.0)
            .radius(6.0)
            .border(1.5, p.k.c.border_strong),
        CheckState::Running => Spinner::new().size(14.0).color(p.accent()).build(p.k),
        CheckState::Ok => icon(PASSED, 16.0).color(p.success()),
        CheckState::Failed => icon(FAILED, 16.0).color(p.danger()),
    };
    el().key("mark")
        .size(16.0, 16.0)
        .center()
        .child(mark)
        .a11y_hidden()
}

