//! Work the setup flow and Settings start that blocks (finding the agents,
//! asking agent-desktop, installing, the self-tests, the Keychain, the
//! voices) runs on its own thread; each answer wakes the window and lands
//! in the next poll. In scenes and tests nothing runs: each job is
//! recorded, and a test answers it with [`Jobs::finish`].

use std::sync::mpsc::{Receiver, Sender, channel};

use rune::Waker;

use super::Ui;
use super::shortcut::{self, Conflict};
use crate::app_settings::{AppSettings, AssistantSettings, HotkeySlot};
use crate::assistant::backends::{AgentDesktop, CheckResult, Fix, Found};
use crate::assistant::computer::Permissions;

/// Something to find out or do off the window's thread.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::rune_ui) enum Job {
    /// Which agents this Mac has, looked for afresh (`backends::detect`).
    Detect,
    /// agent-desktop and its permissions (`backends::agent_desktop`).
    Desktop,
    /// Hex's own Accessibility and Screen Recording, read afresh.
    Permissions,
    /// What a failed check or row offers (`backends::apply_fix`). Opening
    /// Hex's own System Settings guide runs on the window's thread, where
    /// the guide can show.
    Fix(Fix),
    /// The self-tests for these settings' agent, in order.
    SelfTests(AssistantSettings),
    /// What else takes the tested shortcuts.
    Conflicts(Box<AppSettings>),
    /// Saves the Claude API key in the Keychain (empty deletes it).
    SaveKey(String),
    /// The installed voices.
    Voices,
    /// Reads a line aloud with this voice.
    Speak(Option<String>),
}

/// A job's answer.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::rune_ui) enum Done {
    Detected(Vec<Found>),
    Desktop(AgentDesktop),
    Permissions(Permissions),
    /// A fix ran: done, or why not.
    Fixed(Fix, Result<(), String>),
    /// A self-test started (`Running`) or ended.
    Check(CheckResult),
    AllChecked,
    /// What else takes each tested shortcut.
    Conflicts(Vec<(HotkeySlot, Conflict)>),
    KeySaved(Result<(), String>),
    Voices(Vec<String>),
}

pub(in crate::rune_ui) struct Jobs {
    sender: Sender<Done>,
    answers: Receiver<Done>,
    /// Set once the window connects to the service: jobs really run.
    waker: Option<Waker>,
    /// Jobs asked for while nothing runs them (scenes and tests).
    pub(in crate::rune_ui) asked: Vec<Job>,
}

impl Default for Jobs {
    fn default() -> Self {
        let (sender, answers) = channel();
        Self {
            sender,
            answers,
            waker: None,
            asked: Vec::new(),
        }
    }
}

impl Jobs {
    pub(super) fn connect(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Starts `job` on its own thread; without a connection, records it.
    pub(in crate::rune_ui) fn run(&mut self, job: Job) {
        let Some(waker) = self.waker.clone() else {
            self.asked.push(job);
            return;
        };
        if let Job::Fix(fix @ Fix::OpenSettings(_)) = job {
            // Hex's floating guide shows only from the main thread.
            let result = crate::assistant::backends::apply_fix(fix);
            let _ = self.sender.send(Done::Fixed(
                fix,
                result.map_err(|error| format!("{error:#}")),
            ));
            waker();
            return;
        }
        let sender = self.sender.clone();
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-setup".into())
            .spawn(move || {
                let send = |done: Done| {
                    let _ = sender.send(done);
                    waker();
                };
                perform(job, &send);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start an assistant setup job");
        }
    }

    /// Answers a job as its thread would, for tests.
    #[cfg(test)]
    pub(in crate::rune_ui) fn finish(&self, done: Done) {
        let _ = self.sender.send(done);
    }

    fn take(&self) -> Vec<Done> {
        self.answers.try_iter().collect()
    }
}

/// Does `job`, sending what it found.
fn perform(job: Job, send: &dyn Fn(Done)) {
    use crate::assistant::{backends, computer, credentials, speech};
    match job {
        Job::Detect => {
            backends::refresh();
            send(Done::Detected(backends::detect()));
        }
        Job::Desktop => send(Done::Desktop(backends::agent_desktop())),
        Job::Permissions => send(Done::Permissions(computer::permissions_now())),
        Job::Fix(fix) => send(Done::Fixed(
            fix,
            backends::apply_fix(fix).map_err(|error| format!("{error:#}")),
        )),
        Job::SelfTests(settings) => {
            let checks = backends::Check::for_backend(settings.backend);
            backends::self_tests(&checks, &settings, |result| {
                send(Done::Check(result.clone()))
            });
            send(Done::AllChecked);
        }
        Job::Conflicts(settings) => send(Done::Conflicts(
            super::setup::TESTED
                .iter()
                .flat_map(|slot| {
                    shortcut::conflicts(*slot, &settings)
                        .into_iter()
                        .map(move |conflict| (*slot, conflict))
                })
                .collect(),
        )),
        Job::SaveKey(key) => send(Done::KeySaved(
            credentials::set_api_key(&key).map_err(|error| format!("{error:#}")),
        )),
        Job::Voices => send(Done::Voices(speech::voices())),
        Job::Speak(voice) => speech::speak(
            crate::desktop::i18n::tr("Hi, I'm your assistant. Ask me anything."),
            voice.as_deref(),
        ),
    }
}


/// Takes in every answer that arrived. True when any did.
pub(super) fn poll(ui: &mut Ui) -> bool {
    let answers = ui.chats.jobs.take();
    let changed = !answers.is_empty();
    for done in answers {
        receive(ui, done);
    }
    changed
}

/// One answer, wherever it shows: the setup flow, and Settings' lists.
pub(in crate::rune_ui) fn receive(ui: &mut Ui, done: Done) {
    match &done {
        Done::Detected(found) => ui.chats.settings.found = Some(found.clone()),
        Done::Voices(voices) => ui.chats.settings.voices = Some(voices.clone()),
        Done::KeySaved(Ok(())) => {
            ui.chats.settings.key.clear();
            ui.chats.settings.key_error = None;
            // Ready follows the saved key.
            super::setup::detect(ui);
        }
        Done::KeySaved(Err(error)) => ui.chats.settings.key_error = Some(error.clone()),
        _ => {}
    }
    super::setup::receive(ui, done);
}
