//! The onboarding's Browser page: agent-browser found or installed with one
//! click (its own Chrome with it), its self-tests, and signing in to sites
//! in Hex's own browser, which is optional.

use rune::{El, el, row, text};

use super::super::super::controls::Paint;
use super::super::super::lists::small_button;
use super::super::{Job, Ui};
use super::super::setup::{self, Page, SetupFlow, check_title, page_done};
use super::checks::result_row;
use super::desktop::install_failure;
use super::{GLOBE, TOOL, actions, done_pill, lead, page, push, row_of, rows_card, spinner};
use crate::assistant::backends::{CheckState, Fix};
use crate::desktop::i18n::{tr, tr_fill};

/// A spinner with the install's latest line beside it.
pub(super) fn installing(p: Paint, flow: &SetupFlow, fallback: &str) -> (String, El) {
    let line = flow
        .fix_log
        .last()
        .cloned()
        .unwrap_or_else(|| fallback.to_owned());
    (line, spinner(p, "installing"))
}

/// Sign in to sites…, then Done signing in while Hex's browser shows.
pub(in crate::rune_ui) fn sign_in_button(ui: &Ui, p: Paint, key: &str) -> El {
    let settings = &ui.chats.settings;
    if settings.signing_in {
        small_button(p, key, tr("Done signing in"), |ui: &mut Ui, _| {
            ui.chats.jobs.run(Job::FinishSignIn);
        })
    } else {
        small_button(p, key, tr("Sign in to sites…"), |ui: &mut Ui, _| {
            ui.chats.settings.sign_in_error = None;
            ui.chats.jobs.run(Job::SignIn(None));
        })
    }
}

/// agent-browser: found or installed, its browser downloaded, its
/// self-tests, and the optional sign-in.
pub(super) fn browser(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let step = &flow.browser;
    let found = step.found.as_ref();
    let busy = flow.fixing == Some(Fix::InstallAgentBrowser);
    let install = |primary: bool| {
        push(p, "install", tr("Install"), primary, |ui: &mut Ui| {
            setup::apply(ui, Fix::InstallAgentBrowser)
        })
    };
    let (detail, control) = match found {
        _ if busy => installing(p, flow, tr("Installing… this takes a minute or two")),
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
        } else if busy {
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
            &GLOBE,
            tr("Its own browser"),
            &detail,
            p.muted(),
            control,
        ));
        for (index, result) in step.checks.iter().enumerate() {
            let control = match result.fix {
                Some(Fix::Retry) if result.state == CheckState::Failed => {
                    if step.checks_running {
                        el().key("none")
                    } else {
                        push(p, "fix", tr("Try again"), false, |ui: &mut Ui| {
                            setup::browser::run_checks(ui)
                        })
                    }
                }
                Some(fix) if result.state == CheckState::Failed && !busy => push(
                    p,
                    "fix",
                    &setup::fix_label(fix),
                    false,
                    move |ui: &mut Ui| setup::apply(ui, fix),
                ),
                _ => el().key("none"),
            };
            rows.push(result_row(
                p,
                format!("browser-check{index}"),
                check_title(result.check),
                result.state,
                &result.detail,
                control,
            ));
        }
        if step.ready() {
            let signed_in = ui.chats.settings.signed_in;
            rows.push(row_of(
                p,
                "sign-in",
                super::KEY,
                tr("Sign in to sites (optional)"),
                if signed_in {
                    tr("Tasks start signed in where you signed in")
                } else {
                    tr("In Hex's own browser window. Tasks then start signed in there")
                },
                p.muted(),
                sign_in_button(ui, p, "sign-in-sites"),
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
        flow.fix_error
            .as_ref()
            .map(|error| install_failure(p, error, flow.fix_command.as_deref()))
            .or_else(|| {
                ui.chats.settings.sign_in_error.as_ref().map(|error| {
                    row()
                        .key("sign-in-error")
                        .child(super::note(p, error))
                })
            })
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
