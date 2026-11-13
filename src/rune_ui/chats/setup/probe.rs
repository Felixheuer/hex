//! The Shortcut page: other apps and macOS features on the tested
//! shortcuts, and a press test of each (`recognition::shortcut_probe`).

use super::*;

/// The shortcuts the Shortcut page tests: the Action shortcut, which the
/// page waits for, and dictation's beside it.
pub(in crate::rune_ui) const TESTED: [HotkeySlot; 2] = [HotkeySlot::Action, HotkeySlot::Dictation];

/// How long a press test waits for its press.
const PROBE_WAIT: Duration = Duration::from_secs(20);

/// Looks for other apps and macOS features on the tested shortcuts.
pub(in crate::rune_ui) fn check_shortcut(ui: &mut Ui) {
    let settings = ui.settings.clone();
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    flow.conflicts_checked_at = Some(Instant::now());
    if !flow.conflicts_checking {
        flow.conflicts_checking = true;
        ui.chats.jobs.run(Job::Conflicts(Box::new(settings)));
    }
}

/// Asks for a press of `slot`'s shortcut now: the row waits, then shows
/// whether its press and release really reached Agent-Desk.
pub(in crate::rune_ui) fn test_shortcut(ui: &mut Ui, slot: HotkeySlot) {
    let live = ui.chats.live();
    stop_probe(ui);
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    flow.probes.retain(|(tested, _)| *tested != slot);
    flow.probes.push((slot, ProbeState::Waiting));
    if live {
        flow.probing = Some((slot, shortcut::start_probe(slot, PROBE_WAIT)));
    }
}

/// Ends a press test still waiting; its row goes back to Test.
pub(super) fn stop_probe(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    if let Some((slot, _)) = flow.probing.take() {
        shortcut::cancel_probe();
        flow.probes.retain(|(tested, _)| *tested != slot);
    }
    flow.probes
        .retain(|(_, state)| *state != ProbeState::Waiting);
}

/// Opens where a conflict is fixed; the page reads them again meanwhile.
pub(in crate::rune_ui) fn clear_conflict(ui: &mut Ui, fix: &ConflictFix) {
    if ui.chats.live() {
        fix.open();
    }
}

/// Another shortcut, in Assistant settings.
pub(in crate::rune_ui) fn choose_another(ui: &mut Ui, tx: &mut rune::Tx) {
    close(ui);
    ui.select_pane(super::super::super::Pane::Assistant, tx);
}

/// The press test's state for `slot`.
pub(in crate::rune_ui) fn probe_state(flow: &SetupFlow, slot: HotkeySlot) -> Option<&ProbeState> {
    flow.probes
        .iter()
        .find(|(tested, _)| *tested == slot)
        .map(|(_, state)| state)
}
