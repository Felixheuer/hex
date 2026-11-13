//! Settings rows for the person's accounts in Agent-Desk's hidden browsers: take
//! the logins of a Chrome profile over (refreshed daily), sign in by hand,
//! forget them all, and whether new browsers start with them
//! (`assistant::browser::accounts`). Self-contained: whichever settings pane
//! shows the assistant's Web section calls [`rows`] (and [`read`] when it
//! opens).

use std::sync::Mutex;

use rune::El;

use super::super::Menu;
use super::super::controls::{CardRow, Paint, described_row, note, toggle_row};
use super::super::general::quiet_select;
use super::super::lists::small_button;
use super::Ui;
use super::jobs::Job;
use crate::assistant::browser::accounts::{Imported, Profile};
use crate::desktop::i18n::{tr, tr_fill};

/// What the rows show, as the last read or import left it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::rune_ui) struct Accounts {
    /// Chrome's profiles, once read.
    pub profiles: Vec<Profile>,
    /// The last import from Chrome.
    pub last: Option<Imported>,
    /// An import runs now.
    pub importing: bool,
    /// Why the last import failed.
    pub error: Option<String>,
}

static STATE: Mutex<Option<Accounts>> = Mutex::new(None);

/// Why Google and YouTube don't come over (`accounts::kept_in_chrome`).
const GOOGLE_STAYS: &str = "Google and YouTube stay signed in only in Chrome: Google signs you out everywhere when its login is used in a second browser. Sign in to them by hand if tasks need them";

fn state() -> Accounts {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .unwrap_or_default()
}

fn set(update: impl FnOnce(&mut Accounts)) {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    update(state.get_or_insert_with(Accounts::default));
}

/// Reads Chrome's profiles and the last import (off the window's thread).
pub(in crate::rune_ui) fn read(ui: &mut Ui) {
    ui.chats.jobs.run(Job::Accounts);
    ui.chats.jobs.run(Job::Browser);
}

/// What [`Job::Accounts`] or [`Job::ImportChrome`] found.
pub(in crate::rune_ui) fn receive(found: &Accounts) {
    set(|state| *state = found.clone());
}

/// The rows: accounts note, Chrome import, sites, manual sign-in and
/// forgetting, and whether new browsers start signed in.
pub(in crate::rune_ui) fn rows(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<CardRow> {
    let accounts = state();
    let settings = &ui.settings.assistant;
    let mut rows: Vec<CardRow> = vec![
        note(
            "accounts-note",
            tr("Agent-Desk's browsers then use your accounts: tasks can read and act as you on these sites"),
            p.muted(),
        )
        .into(),
    ];

    // The Chrome profile to take logins from.
    let chosen = settings.browser_import_profile.clone();
    let mut labels = vec![tr("None").to_owned()];
    labels.extend(
        accounts
            .profiles
            .iter()
            .map(|profile| format!("{} ({})", profile.name, profile.directory)),
    );
    let selected = chosen
        .as_deref()
        .and_then(|dir| {
            accounts
                .profiles
                .iter()
                .position(|profile| profile.directory == dir)
        })
        .map_or(0, |index| index + 1);
    let directories: Vec<String> = accounts
        .profiles
        .iter()
        .map(|profile| profile.directory.clone())
        .collect();
    let picker = quiet_select(
        ui,
        p,
        Menu::Pick("accounts-chrome-profile"),
        "accounts-chrome-profile-control".to_owned(),
        tr("Chrome profile"),
        labels,
        selected,
        move |ui, chosen| {
            let profile = chosen
                .checked_sub(1)
                .and_then(|index| directories.get(index))
                .cloned();
            ui.update(|settings| settings.assistant.browser_import_profile = profile);
        },
        layer,
    );
    rows.push(
        described_row(
            p,
            "accounts-chrome-profile",
            tr("Take logins from Chrome"),
            Some((
                tr("Taken again every day. macOS asks once whether Agent-Desk may read Chrome's key")
                    .to_owned(),
                p.muted(),
            )),
            picker,
        )
        .into(),
    );

    let (status, ink) = import_status(&accounts, p);
    let importing_from = chosen.is_some();
    let import: El = match chosen {
        Some(profile) if !accounts.importing => small_button(
            p,
            "accounts-import",
            tr("Take over now"),
            move |ui: &mut Ui, _| {
                set(|state| {
                    state.importing = true;
                    state.error = None;
                });
                ui.chats.jobs.run(Job::ImportChrome(profile.clone()));
            },
        ),
        Some(_) => rune_kit::Spinner::new()
            .size(14.0)
            .color(p.muted())
            .build(p.k)
            .key("accounts-importing"),
        None => rune::el().key("none"),
    };
    rows.push(
        described_row(
            p,
            "accounts-import-row",
            tr("Logins from Chrome"),
            Some((status, ink)),
            import,
        )
        .into(),
    );
    if importing_from {
        rows.push(note("accounts-google-note", tr(GOOGLE_STAYS), p.muted()).into());
    }

    // Signed-in sites, sign in by hand, forget all.
    let sites = &ui.chats.settings.saved_sites;
    let summary = if sites.is_empty() {
        tr("No accounts yet").to_owned()
    } else {
        sites_summary(sites)
    };
    let manual = if ui.chats.settings.signing_in {
        small_button(p, "accounts-done", tr("Done"), |ui: &mut Ui, _| {
            ui.chats.jobs.run(Job::FinishSignIn);
        })
    } else {
        small_button(
            p,
            "accounts-sign-in",
            tr("Sign in by hand…"),
            |ui: &mut Ui, _| {
                ui.chats.jobs.run(Job::SignIn(None));
            },
        )
    };
    rows.push(
        described_row(
            p,
            "accounts-sites",
            tr("Signed-in sites"),
            Some((summary, p.muted())),
            manual,
        )
        .into(),
    );
    if !sites.is_empty() {
        rows.push(
            described_row(
                p,
                "accounts-forget",
                tr("Forget all logins"),
                Some((
                    tr("New browsers start signed out until you take logins over again").to_owned(),
                    p.muted(),
                )),
                small_button(
                    p,
                    "accounts-forget-button",
                    tr("Forget"),
                    |ui: &mut Ui, _| {
                        ui.chats.jobs.run(Job::ForgetSignIn);
                    },
                ),
            )
            .into(),
        );
    }
    rows.push(
        toggle_row(
            p,
            "accounts-default",
            tr("New browsers start signed in"),
            tr("Every task's browser starts with your accounts; a task can still ask for a clean one"),
            settings.browser_signed_in_by_default,
            |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.browser_signed_in_by_default ^= true);
            },
        )
        .into(),
    );
    rows
}

fn import_status(accounts: &Accounts, p: Paint) -> (String, rune::Color) {
    if accounts.importing {
        return (
            tr("Taking your logins over from Chrome…").to_owned(),
            p.accent(),
        );
    }
    if let Some(error) = &accounts.error {
        return (error.clone(), p.danger());
    }
    match &accounts.last {
        Some(last) => (
            tr_fill(
                "{} sites taken over from Chrome",
                &last.sites.len().to_string(),
            ),
            p.muted(),
        ),
        None => (tr("Not taken over yet").to_owned(), p.muted()),
    }
}

/// "github.com, google.com and 12 more".
fn sites_summary(sites: &[String]) -> String {
    const SHOWN: usize = 4;
    let shown = sites
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if sites.len() > SHOWN {
        format!(
            "{shown} {}",
            tr_fill("and {} more", &(sites.len() - SHOWN).to_string())
        )
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_site_list_is_shortened() {
        let sites: Vec<String> = (0..7).map(|n| format!("s{n}.com")).collect();
        let summary = sites_summary(&sites);
        assert!(summary.starts_with("s0.com, s1.com, s2.com, s3.com"));
        assert!(summary.contains('3'));
        assert_eq!(sites_summary(&sites[..2]), "s0.com, s1.com");
    }
}
