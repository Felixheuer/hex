//! The onboarding's self-test pages: Checks (the agent answers and can use
//! agent-desktop) and Browser (agent-browser found or installed, its own
//! browser downloaded, and its own self-tests). A failed check always shows
//! its fix, even while later checks still run.

use rune::{El, col, el, row, text};
use rune_kit::{Button, ButtonVariant};

use super::super::super::controls::{Paint, PushSize};
use super::super::Ui;
use super::super::browser_shim as bshim;
use super::super::setup::{self, Page, SetupFlow, check_title, fix_label, page_done};
use super::{TOOL, actions, done_pill, lead, mark, note, page, push, row_of, rows_card, spinner};
use crate::assistant::backends::{CheckResult, CheckState, Fix};
use crate::desktop::i18n::{tr, tr_fill};

/// One self-test's row: its mark, what it proves over what it found, and
/// its fix when it failed.
#[allow(clippy::too_many_arguments)]
fn result_row(
    p: Paint,
    key: String,
    title: String,
    state: CheckState,
    detail: &str,
    control: El,
) -> El {
    let ink = if state == CheckState::Failed {
        p.danger()
    } else {
        p.muted()
    };
    let said = match state {
        CheckState::Pending => tr("Waiting to run").to_owned(),
        CheckState::Running if detail.is_empty() => tr("Checking…").to_owned(),
        _ => detail.to_owned(),
    };
    row()
        .key(key)
        .role("group")
        .label(title.clone())
        .description(said.clone())
        .min_h(48.0)
        .py(8.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .child(mark(p, state))
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

/// A failed check's fix: its button at once, even while later checks run
/// (only Try again waits for them), and a spinner while it works.
fn fix_control(flow: &SetupFlow, p: Paint, result: &CheckResult) -> El {
    match result.fix {
        Some(fix) if result.state == CheckState::Failed && flow.fixing == Some(fix) => {
            spinner(p, "fixing")
        }
        Some(Fix::Retry) if flow.checks_running => el().key("none"),
        Some(fix) if result.state == CheckState::Failed => {
            push(p, "fix", &fix_label(fix), false, move |ui: &mut Ui| {
                setup::apply(ui, fix)
            })
        }
        _ => el().key("none"),
    }
}

/// The live self-tests, run on arrival; Run again after a fix.
pub(super) fn checks(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let rows = flow
        .checks
        .iter()
        .enumerate()
        .map(|(index, result)| {
            result_row(
                p,
                format!("check{index}"),
                check_title(result.check),
                result.state,
                &result.detail,
                fix_control(flow, p, result),
            )
        })
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

/// A browser self-test's row title.
fn browser_check_title(check: bshim::Check) -> String {
    match check {
        bshim::Check::BrowserRoundTrip => tr("agent-browser opens a page in a hidden browser").into(),
        bshim::Check::BrowserThroughAgent(backend) => tr_fill(
            "{} can use the hidden browser",
            super::super::backend_name(backend),
        ),
    }
}

/// agent-browser: found or installed, its browser downloaded, and its
/// self-tests.
pub(super) fn browser(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let step = &flow.browser;
    let found = step.found.as_ref();
    let install = |primary: bool| {
        push(p, "install", tr("Install"), primary, |ui: &mut Ui| {
            setup::browser::install(ui)
        })
    };
    let (detail, control) = match found {
        _ if step.installing => (
            tr("Installing with Homebrew… this takes a minute or two").to_owned(),
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
            tr("Opens web pages in browsers of its own, out of sight").to_owned(),
            install(true),
        ),
    };
    let mut rows = vec![row_of(
        p,
        "agent-browser",
        TOOL,
        "agent-browser",
        &detail,
        p.muted(),
        control,
    )];
    if let Some(found) = found.filter(|found| found.path.is_some()) {
        let (detail, control) = if found.browser_ready {
            (
                tr("Downloaded, separate from your own browser").to_owned(),
                done_pill(p, "downloaded", tr("Ready")),
            )
        } else if step.installing {
            (tr("Downloading…").to_owned(), spinner(p, "downloading"))
        } else {
            (
                tr("Its own copy of Chrome, separate from yours").to_owned(),
                install(true),
            )
        };
        rows.push(row_of(
            p,
            "browser",
            &super::GLOBE,
            tr("Its own browser"),
            &detail,
            p.muted(),
            control,
        ));
        for (index, result) in step.checks.iter().enumerate() {
            let control = match result.fix {
                Some(fix) if result.state == CheckState::Failed && !step.installing => {
                    let label = match fix {
                        bshim::Fix::InstallAgentBrowser => tr("Install"),
                        bshim::Fix::Retry => tr("Try again"),
                    };
                    push(p, "fix", label, false, move |ui: &mut Ui| {
                        setup::browser::apply(ui, fix)
                    })
                }
                _ => el().key("none"),
            };
            rows.push(result_row(
                p,
                format!("browser-check{index}"),
                browser_check_title(result.check),
                result.state,
                &result.detail,
                control,
            ));
        }
    }
    let below = if !ui.settings.assistant.background_browser {
        Some(
            text(tr(
                "The hidden browser is off in Settings, so web tasks use your own browser",
            ))
            .key("note")
            .font(p.k.ty.small)
            .color(p.muted())
            .wrap()
            .max_w(super::COL_W),
        )
    } else {
        step.error.as_ref().map(|error| note(p, error))
    };
    page(
        p,
        "browser",
        tr("A browser of its own"),
        lead(
            p,
            tr("Web tasks run in agent-browser's hidden browsers, one per task, so many run at once and none takes over your windows."),
        ),
        Some(rows_card(p, "browser-card", rows)),
        below,
        actions(p, page_done(ui, flow, Page::Browser), true),
    )
}

