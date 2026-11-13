//! The assistant's onboarding as it draws: the first run's full-window
//! page, type, card, rows, and motion. One page at a time: its heading, a
//! card of live rows, and Continue (once the page works) beside Later. The
//! self-test pages (`checks`), the shortcut page (`shortcut`), and the
//! gesture tutorial (`try_it`) live beside it.

mod browser;
mod checks;
mod desktop;
mod done;
mod permissions;
mod privacy;
mod shortcut;
mod try_it;

pub(super) use desktop::install_failure;

use rune::{Align, El, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Spinner, glyphs};

use super::super::controls::{Paint, PushSize, pulsing_dot};
use super::super::icons::{GLOBE, MIC, SPARKLES};
use super::super::onboarding_kit::{LEAD, Motion, arrives, keys, title_font};
use super::Ui;
use super::setup::{self, Page, SetupFlow, page_done};
use crate::assistant::backends::CheckState;
use crate::desktop::i18n::{tr, tr_fill};

const TOP_BAR: f32 = 48.0;
const ACTION_H: f32 = 32.0;
/// The column the card fills, as wide as the first run's.
const COL_W: f32 = 520.0;
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
        Page::Permissions => permissions::permissions(ui, flow, p),
        Page::Agent => agent(ui, flow, p),
        Page::Desktop => desktop::desktop(ui, flow, p),
        Page::Browser => browser::browser(ui, flow, p),
        Page::Shortcut => shortcut::shortcut(ui, flow, p),
        Page::Try => try_it::try_it(ui, flow, p),
        Page::Done => done::done(ui, flow, p),
    };
    let page = col()
        .key("step")
        .gap(10.0)
        .items_center()
        .child(step_line(flow, p))
        .child(page);
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
            .children(repair_banner(flow, p))
            .children(flow.from_hold.then(|| {
                row().key("from-hold").w_full().justify_center().child(
                    text(tr("Finish setting up the assistant first"))
                        .key("words")
                        .role("status")
                        .font(p.k.ty.small_medium)
                        .color(p.accent()),
                )
            }))
            .child(
                super::super::scroll_body(
                    p.k,
                    "assistant-setup-scroll",
                    col()
                        .key("middle")
                        .w_full()
                        .items_center()
                        .py(32.0)
                        .child(page),
                )
                .grow(1.0)
                .shrink(),
            )
            .modal(true, setup::close),
    )
}

/// The guide's header, when setup came back for what's missing: what
/// changed, one line each.
fn repair_banner(flow: &SetupFlow, p: Paint) -> Option<El> {
    let repair = flow.repair.as_ref()?;
    let mut said: Vec<&'static str> = Vec::new();
    for gap in &repair.gaps {
        let line = gap.message();
        if !said.contains(&line) {
            said.push(line);
        }
    }
    let k = p.k;
    let notice = col()
        .key("repair-notice")
        .w(COL_W)
        .px(16.0)
        .py(12.0)
        .gap(6.0)
        .radius(super::super::controls::CARD_RADIUS)
        .fill(k.alpha(p.warning(), 0.10))
        .border(p.hair, k.alpha(p.warning(), 0.35))
        .child(
            text(tr("Agent-Desk needs your help for a moment"))
                .key("repair-title")
                .font(k.ty.ui_medium)
                .color(p.text()),
        )
        .children(said.into_iter().enumerate().map(|(index, line)| {
            row()
                .key(format!("repair-gap{index}"))
                .gap(8.0)
                .items_start()
                .child(
                    el().size(5.0, 5.0)
                        .mt(6.0)
                        .radius(2.5)
                        .fill(p.warning())
                        .a11y_hidden(),
                )
                .child(
                    text(tr(line))
                        .key("words")
                        .font(k.ty.small)
                        .color(p.muted())
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                )
        }));
    Some(
        col()
            .key("repair-banner")
            .role("status")
            .w_full()
            .items_center()
            .pt(8.0)
            .child(notice),
    )
}

/// Back on the left, where the flow is in the middle, Close on the right.
/// The bar moves the window, as the first run's does.
fn top_bar(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let back = setup::repair::previous_page(flow, flow.page)
        .is_some()
        .then(|| {
            Button::new(tr("Back"))
                .variant(ButtonVariant::Ghost)
                .on_press(setup::back)
                .build_push(p.k)
                .key("back")
                .absolute(
                    super::super::sidebar::onboarding_back_x(ui.platform.caps.os),
                    8.0,
                )
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

/// One segment per page, as a bar: finished ones green, the current one in
/// full ink, ones left with Later amber, the rest a faint track.
fn progress(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let c = &p.k.c;
    let pages = setup::repair::pages(flow);
    let current = pages
        .iter()
        .position(|page| *page == flow.page)
        .unwrap_or(0);
    let mut marks = row()
        .key("progress")
        .role("progressbar")
        .label(tr("Setup"))
        .a11y_value(tr_fill(
            "Step {}",
            &format!("{} / {}", current + 1, pages.len()),
        ))
        .gap(4.0)
        .center_y();
    let width = (SEGMENTS_W / pages.len().max(1) as f32).clamp(14.0, 40.0);
    for (index, page) in pages.iter().enumerate() {
        let here = index == current;
        let fill = if here {
            c.text_base
        } else if flow.later.contains(page) {
            p.warning()
        } else if index < current && page_done(ui, flow, *page) {
            p.success()
        } else if index < current {
            c.text_faint
        } else {
            c.border_base
        };
        marks = marks.child(
            el().key(format!("mark{index}"))
                .size(width, 4.0)
                .radius(2.0)
                .fill(fill)
                .motion(rune::Spring::new(0.3, 0.0))
                .a11y_hidden(),
        );
    }
    marks
}

/// How wide the progress bar's segments are together.
const SEGMENTS_W: f32 = 240.0;

/// "Step 2 of 8 · Permissions" over the page's title; back for what's
/// missing, "1 of 2 to fix".
fn step_line(flow: &SetupFlow, p: Paint) -> El {
    let pages = setup::repair::pages(flow);
    let current = pages
        .iter()
        .position(|page| *page == flow.page)
        .unwrap_or(0);
    let count = [
        ("n", (current + 1).to_string()),
        ("of", pages.len().to_string()),
    ];
    let count: Vec<(&str, &str)> = count.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let words = if flow.repair.is_some() {
        crate::desktop::i18n::tr_named("{n} of {of} to fix", &count)
    } else {
        crate::desktop::i18n::tr_named("Step {n} of {of}", &count)
    };
    let mut words = format!("{words} · {}", flow.page.step_name());
    if flow.page.optional() {
        words = format!("{words} · {}", tr("Optional"));
    }
    text(words)
        .key("step-line")
        .font(p.k.ty.small_medium)
        .color(p.faint())
        .align(Align::Center)
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
                    .color(p.text())
                    .max_w(COL_W)
                    .align(Align::Center)
                    .wrap(),
            )
            .child(lead),
    );
    col()
        .key(key.to_owned())
        .gap(24.0)
        .items_center()
        .child(heading)
        .children(card.map(|card| motion.staggered(card, 1.0, 8.0)))
        .children(note)
        .child(actions)
}

/// A small muted heading over a card ("Making sure it works").
fn section_label(p: Paint, key: &str, label: &str) -> El {
    text(label.to_owned())
        .key(key.to_owned())
        .role("heading")
        .level(2)
        .font(p.k.ty.small_medium)
        .color(p.muted())
        .px(4.0)
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

/// The setup's card: the window's card material (the overlay's anatomy:
/// hairline, one soft shadow), a column wide.
fn card(p: Paint, key: &str) -> El {
    super::super::controls::material(p, col().key(key.to_owned()).w(COL_W).py(4.0))
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
    let lead = el()
        .key("icon")
        .size(28.0, 28.0)
        .center()
        .radius(8.0)
        .fill(p.k.c.surface_raised)
        .child(icon(glyph, 15.0).color(p.k.c.icon_base))
        .a11y_hidden();
    row_led(p, key, lead, title, detail, ink, control)
}

/// A checklist row: its glyph on a quiet tile, a green check on a green
/// tint once it's done, then its title over what it found, and a control.
fn status_row(
    p: Paint,
    key: &str,
    glyph: &'static rune::Icon,
    done: bool,
    title: &str,
    detail: &str,
    control: El,
) -> El {
    let k = p.k;
    let lead = if done {
        el().key("icon")
            .size(28.0, 28.0)
            .center()
            .radius(14.0)
            .fill(k.alpha(p.success(), 0.14))
            .child(icon(k.glyph.check_small, 16.0).color(p.success()))
            .transition(arrives())
    } else {
        el().key("icon")
            .size(28.0, 28.0)
            .center()
            .radius(14.0)
            .fill(k.c.surface_raised)
            .border(p.hair, k.c.border_weak)
            .child(icon(glyph, 14.0).color(k.c.icon_base))
    }
    .a11y_hidden();
    row_led(p, key, lead, title, detail, p.muted(), control)
}

fn row_led(
    p: Paint,
    key: &str,
    lead: El,
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
        .py(10.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(lead)
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
            &tr_fill("Hold {} and give it a bigger job", &shortcut),
            tr(
                "It decides on its own to work in the background. Many jobs run at once while a frame circles the screen",
            ),
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
            tr("Assistant"),
            tr("Everything you asked and every job, in one stream in Agent-Desk's window"),
            p.muted(),
            el().key("none"),
        ),
    ];
    let flow = ui.chats.setup.as_ref();
    let (title, copy) = if flow.is_some_and(|flow| flow.merged) {
        (
            tr("Now, your assistant"),
            if cfg!(target_os = "macos") {
                tr(
                    "Agent-Desk can also answer questions about what's on screen and use your Mac for you. A few checks, and you're done.",
                )
            } else {
                tr(
                    "Agent-Desk can also answer questions about what's on screen and use your computer for you. A few checks, and you're done.",
                )
            },
        )
    } else if flow.is_some_and(|flow| flow.new_in_hex) {
        (
            tr("New in Agent-Desk: your assistant"),
            if cfg!(target_os = "macos") {
                tr(
                    "Ask about what's on screen, or let it use your Mac for you. Agent-Desk makes sure every piece works first.",
                )
            } else {
                tr(
                    "Ask about what's on screen, or let it use your computer for you. Agent-Desk makes sure every piece works first.",
                )
            },
        )
    } else {
        (
            tr("Meet your assistant"),
            if cfg!(target_os = "macos") {
                tr(
                    "Ask anything by voice, or let it use your Mac for you. Agent-Desk makes sure every piece works first.",
                )
            } else {
                tr(
                    "Ask anything by voice, or let it use your computer for you. Agent-Desk makes sure every piece works first.",
                )
            },
        )
    };
    page(
        p,
        "welcome",
        title,
        lead(p, copy),
        Some(rows_card(p, "welcome-card", rows)),
        Some(privacy::facts(p).w(COL_W).px(20.0)),
        actions(p, true, false),
    )
}

/// Who answers: the agent picker (one action per agent, a suggestion when
/// nothing is installed, OpenCode on your own key).
fn agent(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let lead_copy = if flow.detecting && ui.chats.settings.found.is_none() {
        if cfg!(target_os = "macos") {
            tr("Looking for the agents on this Mac…")
        } else {
            tr("Looking for the agents on this computer…")
        }
    } else if super::agents::nothing_installed(ui) {
        tr(
            "Nothing is installed yet. One click sets up the one we suggest, or use your own API key.",
        )
    } else {
        if cfg!(target_os = "macos") {
            tr("It runs on this Mac, signed in with your own account or key.")
        } else {
            tr("It runs on this computer, signed in with your own account or key.")
        }
    };
    page(
        p,
        "agent",
        tr("Choose who answers"),
        lead(p, lead_copy),
        Some(card(p, "agent-card").child(super::agent_picker::picker(ui, p))),
        None,
        actions(p, setup::page_done(ui, flow, Page::Agent), true),
    )
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
