//! What reaches the flow while it shows: the tutorial's gestures, the
//! answers of the jobs it ran, and the grants and probe polled every
//! moment.

use super::*;

/// Follows the tutorial's gestures.
pub(in crate::rune_ui::chats) fn event(ui: &mut Ui, event: &Event) -> bool {
    ui.chats
        .setup
        .as_mut()
        .is_some_and(|flow| flow.tutorial.event(event))
}

/// A job's answer, for the flow.
pub(in crate::rune_ui::chats) fn receive(ui: &mut Ui, done: super::super::Done) {
    let previous_permissions = permissions::fingerprint(ui);
    use super::super::Done;
    if browser::receive(ui, &done) {
        return;
    }
    let fixed = matches!(done, Done::Fixed(..));
    let browser_installed = matches!(
        done,
        Done::Fixed(Fix::InstallAgentBrowser | Fix::InstallLightpanda, _)
    );
    let Some(flow) = ui.chats.setup.as_mut() else {
        return;
    };
    match done {
        // Taken by `repair` before it gets here.
        Done::Gaps(_) => {}
        Done::Detected(_) => flow.detecting = false,
        Done::Desktop(found) => {
            flow.desktop_checking = false;
            flow.desktop = Some(found);
            forget_granted(ui);
        }
        Done::Permissions(read) => {
            flow.permissions_checking = false;
            flow.permissions = Some(read);
            forget_granted(ui);
        }
        Done::Check(result) => {
            if let Some(slot) = flow
                .checks
                .iter_mut()
                .find(|slot| slot.check == result.check)
            {
                *slot = result;
            }
        }
        Done::AllChecked => flow.checks_running = false,
        Done::FixProgress(fix, line) => {
            if flow.fixing == Some(fix) && !line.trim().is_empty() {
                flow.fix_log.push(line);
                let extra = flow.fix_log.len().saturating_sub(FIX_LOG);
                flow.fix_log.drain(..extra);
            }
        }
        Done::FixCommand(_, command) => flow.fix_command = command,
        Done::Fixed(fix, result) => {
            if flow.fixing == Some(fix) {
                flow.fixing = None;
            }
            match result {
                // What it printed last, and why it stopped.
                Err(error) => {
                    let mut tail = std::mem::take(&mut flow.fix_log);
                    tail.push(error);
                    flow.fix_error = Some(tail.join("\n"));
                }
                Ok(()) => {
                    flow.fix_log.clear();
                    flow.fix_command = None;
                }
            }
        }
        Done::Conflicts(found) => {
            flow.conflicts_checking = false;
            flow.conflicts = Some(found);
        }
        Done::KeySaved(_)
        | Done::Voices(_)
        | Done::VoiceProgress(_)
        | Done::VoiceDownloaded(_)
        | Done::RunningApps(_)
        | Done::Accounts(_)
        | Done::Browser(..)
        | Done::SignIn(..)
        | Done::BrowserCheck(_)
        | Done::BrowserChecked
        | Done::AgentProgress(..)
        | Done::AgentCommand(_)
        | Done::AgentSetUp(..)
        | Done::OwnKey(_)
        | Done::CodexKey(_) => {}
    }
    if browser_installed {
        browser::installed(ui);
    }
    if fixed {
        // Installed, started, or signed in: look again.
        detect(ui);
        recheck(ui);
    }
    permissions::received(ui, previous_permissions);
}

/// Granted rows stop waiting.
fn forget_granted(ui: &mut Ui) {
    let Some(flow) = ui.chats.setup.as_ref() else {
        return;
    };
    let granted: Vec<Grant> = flow
        .asked
        .iter()
        .copied()
        .filter(|grant| granted(ui, flow, *grant) == Some(true))
        .collect();
    if let Some(flow) = ui.chats.setup.as_mut() {
        flow.asked.retain(|grant| !granted.contains(grant));
    }
}

/// Whether the gestures the Try page asks for really work: it shows, and
/// Finish could finish. `setup_done` waits for Finish, so without this the
/// page would ask for gestures the assistant then refuses.
pub(in crate::rune_ui) fn gestures_work(ui: &Ui) -> bool {
    ui.chats
        .setup
        .as_ref()
        .is_some_and(|flow| flow.page == Page::Try && can_finish(ui, flow))
}

/// Reads the grants again every moment while a page waits on them, and
/// what the probe heard while the Shortcut page shows.
pub(in crate::rune_ui::chats) fn poll(ui: &mut Ui, tx: &mut rune::Tx) -> bool {
    let live = ui.chats.live();
    crate::assistant::set_trying(live && gestures_work(ui));
    let Some(flow) = ui.chats.setup.as_mut() else {
        return false;
    };
    // A press test's answer.
    if let Some((slot, answer)) = &flow.probing {
        let slot = *slot;
        let done = match answer.try_recv() {
            Ok(found) => Some(found),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Probe::Unavailable(tr("The test stopped").into()))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        };
        if let Some(found) = done {
            flow.probing = None;
            flow.probes.retain(|(tested, _)| *tested != slot);
            flow.probes.push((slot, ProbeState::Done(found)));
            return true;
        }
        tx.wake_after(Duration::from_millis(100));
    }
    // Conflicts are read again while their page shows, so a fix made in
    // System Settings or another app shows.
    if flow.page == Page::Shortcut
        && live
        && flow
            .conflicts_checked_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3))
    {
        check_shortcut(ui);
        tx.wake_after(Duration::from_secs(3));
    }
    // The HUD's double-tap, while the tutorial waits for it.
    if let Some(flow) = ui.chats.setup.as_mut()
        && flow.page == Page::Try
        && !flow.tutorial.tasks
    {
        if flow.tutorial.poll() {
            return true;
        }
        if live {
            tx.wake_after(Duration::from_millis(500));
        }
    }
    let Some(flow) = ui.chats.setup.as_ref() else {
        return false;
    };
    if !ui.chats.live() || !matches!(flow.page, Page::Permissions | Page::Desktop) {
        return false;
    }
    let due = flow.checked_at.is_none_or(|at| at.elapsed() >= RECHECK);
    if due {
        recheck(ui);
    }
    tx.wake_after(RECHECK);
    false
}
