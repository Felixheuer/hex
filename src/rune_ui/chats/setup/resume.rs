//! One onboarding that survives a restart. The assistant's pages follow
//! the voice first run (its "Hex is ready" ends both); someone who set Hex
//! up before the assistant gets them as "New in Hex" at launch. The page
//! the flow is on is saved beside the voice tour's markers
//! (`desktop::onboarding::record_assistant`), so a relaunch, whether macOS
//! asked for it after a grant or Restart Hex and continue did it, reopens
//! the flow on that page and checks it again. Closed unfinished, it waits
//! in Chats instead of reopening.

use super::super::super::onboarding_kit::live;
use super::super::Ui;
use super::{Page, open_at};
use crate::desktop::onboarding::{AssistantProgress, record_assistant, saved_assistant};

/// Whether the assistant still needs its onboarding.
fn pending(ui: &Ui) -> bool {
    crate::assistant::enabled(&ui.settings) && !ui.settings.assistant.setup_done
}

/// The saved page to reopen on, live only.
pub(in crate::rune_ui) fn saved_page(ui: &Ui) -> Option<Page> {
    if !ui.chats.live() {
        return None;
    }
    Page::named(&saved_assistant()?.page)
}

fn save(ui: &Ui, page: Page, dismissed: bool) {
    if ui.chats.live() && !ui.settings.assistant.setup_done {
        record_assistant(Some(&AssistantProgress {
            page: page.name().into(),
            dismissed,
        }));
    }
}

/// The flow moved to `page`.
pub(in crate::rune_ui) fn remember(ui: &Ui, page: Page) {
    save(ui, page, false);
}

/// Closed unfinished: it waits in Chats.
pub(in crate::rune_ui) fn dismiss(ui: &Ui) {
    if let Some(flow) = &ui.chats.setup {
        save(ui, flow.page, true);
    }
}

/// About to relaunch for a grant: reopen right here.
pub(in crate::rune_ui) fn before_restart(ui: &Ui) {
    if let Some(flow) = &ui.chats.setup {
        save(ui, flow.page, false);
    }
}

/// Setup finished: nothing to resume.
pub(in crate::rune_ui) fn forget(ui: &Ui) {
    if ui.chats.live() {
        record_assistant(None);
    }
}

/// At launch, once the window is connected: reopens a flow a relaunch cut
/// short where it was, or offers the assistant as "New in Hex" to someone
/// who set Hex up before it. A voice first run under way brings the pages
/// itself ([`follow_first_run`]).
pub(in crate::rune_ui) fn launch(ui: &mut Ui) {
    if !pending(ui) || ui.chats.setup.is_some() || ui.first_run.is_some() {
        return;
    }
    match saved_assistant() {
        Some(progress) if !progress.dismissed => {
            let page = Page::named(&progress.page).unwrap_or(Page::Welcome);
            open_at(ui, page);
            if let Some(flow) = ui.chats.setup.as_mut() {
                flow.resumed = page != Page::Welcome;
            }
        }
        // Closed unfinished: Chats offers to finish.
        Some(_) => {}
        None if crate::desktop::onboarding::completion_recorded() => {
            open_at(ui, Page::Welcome);
            if let Some(flow) = ui.chats.setup.as_mut() {
                flow.new_in_hex = true;
            }
        }
        None => {}
    }
}

/// While the voice first run shows and the assistant still needs setting
/// up, the run holds before "Hex is ready"; once it gets there, the
/// assistant's pages open over it, continuing the same flow. True when
/// they opened.
pub(in crate::rune_ui) fn follow_first_run(ui: &mut Ui) -> bool {
    if ui.first_run.is_none() {
        ui.chats.handed_off = false;
        return false;
    }
    if !pending(ui) || ui.chats.handed_off || ui.chats.setup.is_some() {
        return false;
    }
    if !live::hold_for_assistant(ui) {
        return false;
    }
    ui.chats.handed_off = true;
    let page = saved_page(ui).unwrap_or(Page::Welcome);
    open_at(ui, page);
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.merged = true;
    }
    true
}
