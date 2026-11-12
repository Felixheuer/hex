//! The assistant's onboarding as it draws: the first run's full-window
//! page, type, card, rows, and motion. One page at a time: its heading, a
//! card of live rows, and Continue (once the page works) beside Later.

use rune::{Align, El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, RadioItem, Spinner, TextInput, glyphs};

use super::super::controls::{Paint, PushSize, pulsing_dot};
use super::super::onboarding_kit::{LEAD, Motion, arrives, keys, title_font};
use super::setup::{
    self, Grant, Page, SetupFlow, State, can_finish, check_title, chosen, fix_for, fix_label,
    grant_state, needs_restart, page_done,
};
use super::setup::ProbeState;
use super::shortcut::{ConflictFix, Probe};
use crate::app_settings::HotkeySlot;
use super::{BACKENDS, Ui, backend_name};
use crate::app_settings::AssistantBackend;
use crate::assistant::backends::{CheckResult, CheckState, Fix};
use crate::desktop::i18n::{tr, tr_fill};

const TOP_BAR: f32 = 48.0;
const ACTION_H: f32 = 32.0;
/// The column the card fills, as wide as the first run's.
const COL_W: f32 = 480.0;
const ROW_H: f32 = 56.0;

static PASSED: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_CHECK;
static FAILED: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_X;
static AGENT: &rune::Icon = &rune_theme_opencode::icons::WINDOW_CURSOR;
static CHAT: &rune::Icon = &rune_theme_opencode::icons::SPEECH_BUBBLE;
static SCREEN: &rune::Icon = &rune_theme_opencode::icons::MONITOR;
static TOOL: &rune::Icon = &rune_theme_opencode::icons::CUBE;

/// The flow over the whole window, while it shows.
pub(in crate::rune_ui) fn overlay(ui: &Ui, p: Paint) -> Option<El> {
    let flow = ui.chats.setup.as_ref()?;
    let page = match flow.page {
        Page::Welcome => welcome(ui, p),
        Page::Agent => agent(ui, flow, p),
        Page::Permissions => permissions(ui, flow, p),
        Page::Desktop => desktop(ui, flow, p),
        Page::Checks => checks(ui, flow, p),
        Page::Shortcut => shortcut(ui, flow, p),
        Page::Try => try_it(ui, flow, p),
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

fn welcome(ui: &Ui, p: Paint) -> El {
    let shortcut = ui
        .settings
        .assistant
        .action_hotkey
        .as_ref()
        .map(|binding| super::super::settings::shortcut_caps(binding, false).join(" "))
        .unwrap_or_else(|| tr("the Action shortcut").to_owned());
    let rows = vec![
        row_of(
            p,
            "ask",
            &glyphs::MIC,
            &tr_fill("Hold {} and ask", &shortcut),
            tr("The answer shows beside the HUD and is read aloud"),
            p.muted(),
            el().key("none"),
        ),
        row_of(
            p,
            "agent-mode",
            AGENT,
            &tr_fill("Double-tap {} for Agent mode", &shortcut),
            tr(
                "A frame circles the screen, and every hold starts a task that works beside the others",
            ),
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
    page(
        p,
        "welcome",
        tr("Meet your assistant"),
        lead(
            p,
            tr(
                "Ask anything by voice, or let it use your Mac for you. Hex makes sure every piece works first.",
            ),
        ),
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
    let note = flow
        .fix_error
        .as_ref()
        .or(ui.chats.settings.key_error.as_ref())
        .map(|error| note(p, error));
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

/// A permission's control: Allow, then a waiting dot until macOS reports
/// it on, then Allowed.
fn grant_control(ui: &Ui, flow: &SetupFlow, p: Paint, grant: Grant, primary: bool) -> El {
    match grant_state(ui, flow, grant) {
        State::Passed => done_pill(p, "allowed", tr("Allowed")),
        State::Checking | State::Pending => spinner(p, "reading"),
        State::Waiting if needs_restart(ui, grant) => {
            push(p, "restart", tr("Restart Hex"), true, move |ui: &mut Ui| {
                setup::allow(ui, grant)
            })
        }
        State::Waiting => row()
            .key("waiting-slot")
            .gap(8.0)
            .center_y()
            .child(waiting_pill(p, "waiting", tr("Waiting")))
            .child(push(
                p,
                "again",
                tr("Open again"),
                false,
                move |ui: &mut Ui| setup::allow(ui, grant),
            )),
        State::Failed => push(p, "allow", tr("Allow"), primary, move |ui: &mut Ui| {
            setup::allow(ui, grant)
        }),
    }
}

fn grant_row(ui: &Ui, flow: &SetupFlow, p: Paint, grant: Grant, primary: bool) -> El {
    let (glyph, title, detail) = match grant {
        Grant::Microphone => (
            &glyphs::MIC,
            tr("Microphone"),
            tr("Hears your question while you hold the shortcut"),
        ),
        Grant::Accessibility => (
            &glyphs::CURSOR_TEXT,
            tr("Accessibility"),
            tr("Notices the shortcut and reads the text you selected"),
        ),
        Grant::ScreenRecording => (
            SCREEN,
            tr("Screen Recording"),
            tr("Sends the screen you ask about with your question"),
        ),
        Grant::DesktopAccessibility => (
            &glyphs::CURSOR_TEXT,
            tr("Accessibility"),
            tr("Lets agent-desktop click, type, and read apps"),
        ),
        Grant::DesktopScreenRecording => (
            SCREEN,
            tr("Screen Recording"),
            tr("Lets agent-desktop see windows while a task works"),
        ),
    };
    let state = grant_state(ui, flow, grant);
    let detail = match state {
        State::Waiting if needs_restart(ui, grant) => tr("Turns on when Hex restarts"),
        State::Waiting => tr("Switch on Hex in System Settings"),
        _ => detail,
    };
    row_of(
        p,
        grant.key(),
        glyph,
        title,
        detail,
        p.muted(),
        grant_control(ui, flow, p, grant, primary),
    )
}

/// Hex's own permissions.
fn permissions(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let first_missing = Grant::HEX
        .iter()
        .position(|grant| grant_state(ui, flow, *grant) != State::Passed);
    let rows = Grant::HEX
        .iter()
        .enumerate()
        .map(|(index, grant)| grant_row(ui, flow, p, *grant, Some(index) == first_missing))
        .collect();
    page(
        p,
        "permissions",
        tr("Let Hex see what you see"),
        lead(
            p,
            tr(
                "Each one opens its page in System Settings. This page waits until macOS turns it on.",
            ),
        ),
        Some(rows_card(p, "permissions-card", rows)),
        None,
        actions(p, page_done(ui, flow, Page::Permissions), true),
    )
}

/// agent-desktop: installed, and allowed to act.
fn desktop(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let found = flow.desktop.as_ref();
    let installed = found.is_some_and(|found| found.path.is_some());
    let (detail, control) = match found {
        _ if flow.fixing == Some(Fix::InstallAgentDesktop) => (
            tr("Installing with Homebrew… this takes a minute").to_owned(),
            spinner(p, "installing"),
        ),
        None => (String::new(), spinner(p, "reading")),
        Some(found) if found.path.is_some() => (
            found
                .version
                .as_ref()
                .map(|version| tr_fill("Version {}", version))
                .unwrap_or_default(),
            done_pill(p, "installed", tr("Installed")),
        ),
        Some(_) => (
            tr("Operates apps through Accessibility, without your pointer").to_owned(),
            push(p, "install", tr("Install"), true, |ui: &mut Ui| {
                setup::apply(ui, Fix::InstallAgentDesktop)
            }),
        ),
    };
    let mut rows = vec![row_of(
        p,
        "agent-desktop",
        TOOL,
        "agent-desktop",
        &detail,
        p.muted(),
        control,
    )];
    if installed {
        let first_missing = Grant::DESKTOP
            .iter()
            .position(|grant| grant_state(ui, flow, *grant) != State::Passed);
        rows.extend(
            Grant::DESKTOP
                .iter()
                .enumerate()
                .map(|(index, grant)| grant_row(ui, flow, p, *grant, Some(index) == first_missing)),
        );
    }
    page(
        p,
        "desktop",
        tr("Let the assistant use your Mac"),
        lead(
            p,
            tr("Tasks operate apps through agent-desktop. Hex starts it, so macOS asks for Hex."),
        ),
        Some(rows_card(p, "desktop-card", rows)),
        flow.fix_error.as_ref().map(|error| note(p, error)),
        actions(p, page_done(ui, flow, Page::Desktop), true),
    )
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

fn check_row(ui: &Ui, flow: &SetupFlow, p: Paint, index: usize, result: &CheckResult) -> El {
    let title = check_title(result.check);
    let ink = if result.state == CheckState::Failed {
        p.danger()
    } else {
        p.muted()
    };
    let control = match result.fix {
        Some(fix) if result.state == CheckState::Failed && flow.fixing == Some(fix) => {
            spinner(p, "fixing")
        }
        Some(fix) if result.state == CheckState::Failed && !flow.checks_running => {
            push(p, "fix", &fix_label(fix), false, move |ui: &mut Ui| {
                setup::apply(ui, fix)
            })
        }
        _ => el().key("none"),
    };
    let said = match result.state {
        CheckState::Pending => tr("Waiting to run").to_owned(),
        CheckState::Running if result.detail.is_empty() => tr("Checking…").to_owned(),
        _ => result.detail.clone(),
    };
    let _ = ui;
    row()
        .key(format!("check{index}"))
        .role("group")
        .label(title.clone())
        .description(said.clone())
        .min_h(48.0)
        .py(8.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(mark(p, result.state))
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .child(
                    text(title)
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(p.text())
                        .a11y_hidden(),
                )
                .child(
                    text(said)
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(ink)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
        .child(control)
}

/// The live self-tests, run on arrival; Run again after a fix.
fn checks(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let rows = flow
        .checks
        .iter()
        .enumerate()
        .map(|(index, result)| check_row(ui, flow, p, index, result))
        .collect();
    let done = page_done(ui, flow, Page::Checks);
    let again = (!flow.checks_running && !done).then(|| {
        Button::new(tr("Run again"))
            .variant(ButtonVariant::Neutral)
            .on_press(setup::run_checks)
            .build_push(p.k)
            .key("run-again")
    });
    let actions = actions(p, done, true).children(again);
    page(
        p,
        "checks",
        tr("Making sure it works"),
        lead(
            p,
            tr("Each check sends a real request, so what passes here works for you."),
        ),
        Some(rows_card(p, "checks-card", rows)),
        flow.fix_error.as_ref().map(|error| note(p, error)),
        actions,
    )
}

/// A conflict's fix button.
fn conflict_label(fix: &ConflictFix, name: &str) -> String {
    match fix {
        ConflictFix::KeyboardSettings => tr("Open Keyboard Settings").into(),
        ConflictFix::SiriSettings => tr("Open Siri Settings…").into(),
        ConflictFix::OpenApp(_) => tr_fill("Open {}", name),
    }
}

/// The shortcut's name, as General's rows call it.
fn slot_name(slot: HotkeySlot) -> &'static str {
    match slot {
        HotkeySlot::Action => tr("Action shortcut"),
        _ => tr("Dictation shortcut"),
    }
}

/// The keys of a tested shortcut.
fn slot_keys(ui: &Ui, p: Paint, slot: HotkeySlot) -> Option<El> {
    let binding = match slot {
        HotkeySlot::Action => ui.settings.assistant.action_hotkey.as_ref(),
        _ => Some(&ui.settings.dictation_hotkey),
    }?;
    let caps = super::super::settings::shortcut_caps(binding, false);
    Some(keys(p, "keys", &caps, false))
}

/// What a press test found, for a person; `taker` names who took the keys
/// when a conflict says so.
fn probe_words(found: &Probe, taker: Option<&str>) -> (String, bool) {
    match found {
        Probe::Works(held) => (
            tr_fill("Hex received it (held {})", &format!("{:.1} s", held.as_secs_f32())),
            true,
        ),
        Probe::Swallowed => (
            match taker {
                Some(name) => tr_fill("{} took the keys before Hex could", name),
                None => tr("Another app or macOS took the keys before Hex could").into(),
            },
            false,
        ),
        Probe::NothingReceived => (
            tr("Nothing reached Hex. Another app or macOS may take it").into(),
            false,
        ),
        Probe::WrongKeys(saw) => (tr_fill("Hex saw {} instead", saw), false),
        Probe::NoRelease => (tr("Hex never saw the keys let go").into(), false),
        Probe::Unavailable(why) => (why.clone(), false),
    }
}

/// One shortcut's live press test: Test, a waiting pill while it listens,
/// then what reached Hex.
fn probe_row(ui: &Ui, flow: &SetupFlow, p: Paint, slot: HotkeySlot) -> El {
    let taker = flow.conflicts.as_ref().and_then(|found| {
        found
            .iter()
            .find(|(tested, _)| *tested == slot)
            .map(|(_, conflict)| conflict.name.as_str())
    });
    let (detail, ink, control) = match setup::probe_state(flow, slot) {
        None => (
            tr("Press Test, then press and release it").to_owned(),
            p.muted(),
            push(p, "test", tr("Test"), slot == HotkeySlot::Action, move |ui: &mut Ui| {
                setup::test_shortcut(ui, slot)
            }),
        ),
        Some(ProbeState::Waiting) => (
            tr("Press and release it now").to_owned(),
            p.muted(),
            waiting_pill(p, "waiting", tr("Listening")),
        ),
        Some(ProbeState::Done(found)) => {
            let (words, works) = probe_words(found, taker);
            let control = if works {
                done_pill(p, "works", tr("Works"))
            } else {
                push(p, "test", tr("Test again"), slot == HotkeySlot::Action, move |ui: &mut Ui| {
                    setup::test_shortcut(ui, slot)
                })
            };
            (words, if works { p.muted() } else { p.danger() }, control)
        }
    };
    let title = slot_name(slot);
    row()
        .key(format!("probe-{slot:?}").to_lowercase())
        .role("group")
        .label(title)
        .description(detail.clone())
        .min_h(ROW_H)
        .py(8.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(icon(&glyphs::KEYBOARD, 16.0).key("icon").color(p.k.c.icon_base).a11y_hidden())
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .child(
                    row()
                        .key("title-row")
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(title)
                                .key("title")
                                .font(p.k.ty.ui_medium)
                                .color(p.text())
                                .a11y_hidden(),
                        )
                        .children(slot_keys(ui, p, slot).map(El::a11y_hidden)),
                )
                .child(
                    text(detail)
                        .key("detail")
                        .font(p.k.ty.small)
                        .color(ink)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
        .child(control)
}

/// The shortcuts are free and Hex receives them: what else takes them,
/// each with where it's fixed, then a live press of each.
fn shortcut(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let mut rows = Vec::new();
    match &flow.conflicts {
        None => rows.push(row_of(
            p,
            "conflicts",
            &glyphs::SEARCH,
            tr("Other apps"),
            tr("Looking for apps and macOS features on these shortcuts…"),
            p.muted(),
            spinner(p, "looking"),
        )),
        Some(found) if found.is_empty() => rows.push(row_of(
            p,
            "conflicts",
            &glyphs::SEARCH,
            tr("Other apps"),
            tr("Nothing else on this Mac uses them"),
            p.muted(),
            done_pill(p, "free", tr("Free")),
        )),
        Some(found) => {
            for (index, (slot, conflict)) in found.iter().enumerate() {
                let fix = conflict.fix.clone();
                let control = row()
                    .key("fixes")
                    .gap(8.0)
                    .center_y()
                    .child(push(
                        p,
                        "fix",
                        &conflict_label(&fix, &conflict.name),
                        index == 0 && conflict.taken,
                        move |ui: &mut Ui| setup::clear_conflict(ui, &fix),
                    ))
                    .children((*slot == HotkeySlot::Action && conflict.taken).then(|| {
                        Button::new(tr("Choose another…"))
                            .variant(ButtonVariant::Neutral)
                            .on_press_tx(setup::choose_another)
                            .build_push(p.k)
                            .key("choose-another")
                    }));
                let title = if conflict.taken {
                    tr_fill("{} takes it", &conflict.name)
                } else {
                    tr_fill("{} may take it", &conflict.name)
                };
                rows.push(row_of(
                    p,
                    &format!("conflict{index}"),
                    &glyphs::CIRCLE_X,
                    &format!("{title} · {}", slot_name(*slot)),
                    &conflict.description,
                    if conflict.taken { p.danger() } else { p.warning() },
                    control,
                ));
            }
        }
    }
    for slot in setup::TESTED {
        rows.push(probe_row(ui, flow, p, slot));
    }
    page(
        p,
        "shortcut",
        tr("Make the shortcuts yours"),
        lead(
            p,
            tr("Nothing else may take them, and Hex has to receive them. Test each by pressing it once."),
        ),
        Some(rows_card(p, "shortcut-card", rows)),
        flow.fix_error.as_ref().map(|error| note(p, error)),
        actions(p, page_done(ui, flow, Page::Shortcut), true),
    )
}

/// One real spoken question, then Finish.
fn try_it(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let listening = flow.heard.is_some() && flow.answer.is_none();
    let hold = row()
        .key("hold")
        .gap(8.0)
        .center_y()
        .child(text(tr("Hold")).key("hold").font(LEAD).color(p.muted()))
        .children(action_keys(ui, p, "keys", listening))
        .child(
            text(tr("and ask something"))
                .key("rest")
                .font(LEAD)
                .color(p.muted()),
        );
    let (glyph, title, detail, ink, control) = match (&flow.heard, &flow.answer) {
        (_, Some(Ok(answer))) => (
            CHAT,
            flow.heard.clone().unwrap_or_default(),
            answer.clone(),
            p.text(),
            done_pill(p, "answered", tr("Answered")),
        ),
        (_, Some(Err(why))) => (
            CHAT,
            flow.heard.clone().unwrap_or_default(),
            why.clone(),
            p.danger(),
            push(p, "try-again", tr("Try again"), false, |ui: &mut Ui| {
                if let Some(flow) = ui.chats.setup.as_mut() {
                    flow.heard = None;
                    flow.answer = None;
                }
            }),
        ),
        (Some(heard), None) => (
            CHAT,
            heard.clone(),
            tr("Thinking…").to_owned(),
            p.muted(),
            spinner(p, "answering"),
        ),
        (None, None) => (
            &glyphs::MIC,
            tr("Waiting for your question").to_owned(),
            tr("For example “What time is it in Tokyo?”").to_owned(),
            p.muted(),
            waiting_pill(p, "listening", tr("Listening")),
        ),
    };
    let card = rows_card(
        p,
        "try-card",
        vec![row_of(p, "try", glyph, &title, &detail, ink, control)],
    );
    let agent_hint = row()
        .key("agent-hint")
        .gap(8.0)
        .center_y()
        .child(
            text(tr("Then double-tap"))
                .key("double")
                .font(p.k.ty.small)
                .color(p.faint()),
        )
        .children(action_keys(ui, p, "agent-keys", false))
        .child(
            text(tr(
                "for Agent mode: every hold starts a task that uses your Mac",
            ))
            .key("rest")
            .font(p.k.ty.small)
            .color(p.faint()),
        );
    let finish_ready = can_finish(ui, flow);
    let finish = Button::new(tr("Finish"))
        .variant(ButtonVariant::Contrast)
        .disabled(!finish_ready)
        .on_press_tx(|ui: &mut Ui, tx: &mut Tx| setup::finish(ui, tx))
        .build_push(p.k)
        .key("finish");
    let finish = if finish_ready {
        finish.default_button()
    } else {
        finish
    };
    let blocked = (!finish_ready).then(|| {
        text(tr_fill(
            "Finish waits until {} answers and can use agent-desktop in Checks",
            backend_name(ui.settings.assistant.backend),
        ))
        .key("finish-note")
        .font(p.k.ty.small)
        .color(p.muted())
        .align(Align::Center)
        .wrap()
        .max_w(COL_W)
    });
    let actions = row()
        .key("action")
        .h(ACTION_H)
        .gap(12.0)
        .center()
        .transition(arrives())
        .child(later_button(p))
        .child(finish);
    page(
        p,
        "try",
        tr("Try it"),
        hold,
        Some(card),
        Some(
            col()
                .key("notes")
                .gap(10.0)
                .items_center()
                .child(agent_hint)
                .children(blocked),
        ),
        actions,
    )
}
