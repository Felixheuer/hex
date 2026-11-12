//! Work the setup sheet and Settings start that can block (finding the
//! agents, asking agent-desktop, installing it, the self-tests, the
//! Keychain), run on their own threads; their answers wake the window and
//! land in the next poll. In scenes and tests nothing runs: each job is
//! recorded, and a test answers it with [`Jobs::finish`].

use std::sync::mpsc::{Receiver, Sender, channel};

use rune::Waker;

use super::Ui;
use super::setup::SELF_TESTS;
use crate::app_settings::AssistantBackend;
use crate::assistant::backends::{AgentDesktop, Found};
use crate::assistant::computer::Permissions;

/// Something to find out or do off the window's thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Job {
    /// Which agents this Mac has (`backends::detect`).
    Detect,
    /// agent-desktop and its permissions (`backends::agent_desktop`).
    Desktop,
    /// Hex's own Accessibility and Screen Recording (`computer::permissions`).
    Permissions,
    /// `brew install agent-desktop`.
    Install,
    /// System Settings at agent-desktop's missing permission.
    Guide,
    /// The self-tests, in order, against this backend; those that need
    /// agent-desktop only with `desktop`.
    SelfTests {
        backend: AssistantBackend,
        desktop: bool,
    },
    /// Saves the Claude API key in the Keychain (empty deletes it).
    SaveKey(String),
    /// The installed voices.
    Voices,
    /// Reads a line aloud with this voice.
    Speak(Option<String>),
}

/// A job's answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::rune_ui) enum Done {
    Detected(Vec<Found>),
    Desktop(AgentDesktop),
    Permissions(Permissions),
    Installed(Result<(), String>),
    /// Self-test `index` started.
    TestStarted(usize),
    /// Self-test `index` finished: what it saw, or why it failed.
    Test(usize, Result<String, String>),
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
        Job::Detect => send(Done::Detected(backends::detect())),
        Job::Desktop => send(Done::Desktop(backends::agent_desktop())),
        Job::Permissions => send(Done::Permissions(computer::permissions())),
        Job::Install => send(Done::Installed(
            backends::install_agent_desktop().map_err(|error| format!("{error:#}")),
        )),
        Job::Guide => backends::guide_permissions(),
        Job::SelfTests { backend, desktop } => {
            for (index, check) in SELF_TESTS.iter().enumerate() {
                if check.needs_desktop && !desktop {
                    continue;
                }
                send(Done::TestStarted(index));
                let result = (check.run)(backend).map_err(|error| format!("{error:#}"));
                send(Done::Test(index, result));
            }
        }
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

/// One answer, wherever it shows: the setup sheet, and Settings' lists.
pub(in crate::rune_ui) fn receive(ui: &mut Ui, done: Done) {
    match &done {
        Done::Detected(found) => ui.chats.settings.found = Some(found.clone()),
        Done::Voices(voices) => ui.chats.settings.voices = Some(voices.clone()),
        Done::KeySaved(Ok(())) => {
            ui.chats.settings.key.clear();
            ui.chats.settings.key_error = None;
            // Ready follows the saved key.
            ui.chats.jobs.run(Job::Detect);
        }
        Done::KeySaved(Err(error)) => ui.chats.settings.key_error = Some(error.clone()),
        _ => {}
    }
    super::setup::receive(ui, done);
}
