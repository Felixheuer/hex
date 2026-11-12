//! The assistant's Action shortcut and Stop all tasks, from the
//! listener's side (macOS).
//!
//! The Action machine (`Shortcut::Action`) records exactly as dictation
//! does: the same microphone, HUD, sounds, chord discards, and short-tap
//! discards. Its gestures:
//!
//! - **Hold** is for everything: say anything, and the assistant decides
//!   whether to answer, act in the window in front, or escalate to a
//!   background job. It always asks as `Origin::ActionShortcut`, continuing
//!   `assistant::focus` (a task waiting on a question first, then a running
//!   Here task, then a recent one), so a hold answers an open question or
//!   steers a running task. The desktop is read on its own thread once the
//!   hold is real (past the hold threshold with no other key), so a ⌘C chord
//!   reads nothing; the screenshot leaves out Hex's own windows.
//! - **Tap, then hold** (the second press within the double-tap window, held
//!   past the hold threshold) is an optional shortcut that forces a new
//!   Background task (`Origin::Background`) with no conversation and no
//!   screenshot. It never pastes.
//! - **Double tap** (two taps, no hold) shows or hides the HUD's task list
//!   ([`set_toggle_tasks`]).
//! - A Here question that starts with "im Hintergrund", "in the background",
//!   "nebenbei", "parallel", or "Hintergrund:" goes to the Background lane
//!   instead, without those words ([`spoken_background`]).
//!
//! A hold that becomes real prewarms its lane (`assistant::service::prewarm`).
//! The release hands the clip to transcription as
//! `TranscriptionTarget::Assistant`, processed like a paste but never pasted,
//! copied, kept for Paste last dictation, or saved to History; its own thread
//! then submits the words. Silence, a discard, or a failed transcription asks
//! nothing, and only words for the Here lane stop the answer being read
//! aloud.
//!
//! Escape stops a running Here task and its speech, never a Background one,
//! and Hex takes Escape from the app in front only while one runs or speaks.
//! The Stop all tasks shortcut (`assistant.stop_all_hotkey`, ⌃⌥Esc) stops
//! every task and the speech; the event tap swallows it only while the
//! assistant is on and it is bound (`RuntimeHotkeys::stop_all`).

use color_eyre::Result;

use crate::app_settings::RuntimeHotkey;
use crate::assistant::computer::ContextHandle;
#[cfg(test)]
use crate::assistant::shortcut::spoken_background;
pub(crate) use crate::assistant::shortcut::{Question, set_open_setup, set_toggle_tasks, submit};
use crate::assistant::shortcut::{
    held, here_tasks, open_setup, quick_question_running, toggle_tasks,
};
use crate::audio::CaptureInstant;
use crate::control::{ControlReply, PrintReply};
use crate::desktop::events::EventLog;
use crate::dictation::POST_RELEASE_TAIL;
use crate::dictation::audio::DictationAudio;
use crate::dictation::context::ContextSnapshot;
use crate::feedback::{self, Tone};
use crate::parakeet::DictationWorker;
use crate::suppression::{HotkeyAction, InputEvent};

use super::context_watch::ContextWatch;
use super::{DictationSink, FinishResult};

/// One of the Action machine's actions, and whether the press it starts
/// followed a tap (`Hotkeys::action_press`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Press {
    pub(super) action: HotkeyAction,
    /// A tap came just before: held, this press is a Background task.
    pub(super) after_tap: bool,
}

/// The listener's Action shortcut: the question its press settled, whether
/// the tap keeps Escape for the assistant now, and the Stop all key held.
#[derive(Default)]
pub(super) struct Asker {
    question: Option<Question>,
    /// When a press went down before the assistant was set up: it records
    /// nothing, and held, opens the setup as it comes up.
    setup_press: Option<CaptureInstant>,
    escape: bool,
    /// The Stop all key's code while it is down, so its repeats stop nothing
    /// twice.
    stop_held: Option<u16>,
    /// Conversation mode and barge-in.
    talk: super::talk::Talk,
}

impl Asker {
    /// Carries out one of the Action machine's actions. Returns whether a
    /// start was accepted, as `handle_hotkey_action` does.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn act(
        &mut self,
        press: Press,
        at: CaptureInstant,
        input: &DictationAudio,
        worker: &DictationWorker,
        context: &mut ContextWatch,
        device: &str,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<bool> {
        let handle = |action, context: &mut ContextWatch, events: &mut EventLog| {
            super::handle_hotkey_action(
                action,
                false,
                None,
                at,
                input,
                worker,
                context,
                device,
                events,
                indicator,
                release_while_idle,
            )
        };
        let action = press.action;
        // Not set up yet: the press records nothing, and a hold (never a
        // tap or a ⌘C chord) opens the setup as it comes up.
        if action == HotkeyAction::Start && !crate::app_settings::applied_assistant_ready() {
            self.question = None;
            self.setup_press = Some(at);
            return Ok(true);
        }
        if let Some(pressed) = self.setup_press.take() {
            if action == HotkeyAction::Finish && held(pressed, at) {
                open_setup();
            }
            return Ok(true);
        }
        match action {
            HotkeyAction::Start => {
                // Reads nothing yet: a chord (⌘C) or a short tap is
                // discarded before the hold is real (`intend`).
                let include_screenshot =
                    crate::app_settings::applied_assistant().include_screenshot;
                // One gesture: a hold asks, and the assistant decides
                // whether it answers, works in the background, or starts
                // several jobs (`agent::ROUTING`). A tap before it changes
                // nothing.
                let background = false;
                let _ = press.after_tap;
                self.question = Some(Question::press(background, move || {
                    crate::assistant::computer::context_async(include_screenshot)
                }));
                let accepted = handle(action, context, events)?;
                if accepted {
                    // The HUD wears the assistant's colours (and after a tap
                    // says "New task"), so a question never looks like a
                    // dictation.
                    indicator.send(
                        crate::dictation::indicator::DictationIndicatorEvent::Asking {
                            agent: background,
                        },
                    );
                } else {
                    self.question = None;
                }
                Ok(accepted)
            }
            HotkeyAction::Finish => {
                let page = context.finish(events)?;
                let question = self
                    .question
                    .take()
                    .unwrap_or_else(|| Question::press(false, || ContextHandle::start(|_| {})));
                let follow_up = crate::app_settings::applied_assistant().follow_up_seconds;
                let reply = question.ask(move || crate::assistant::focus(follow_up), submit);
                finish_question(
                    at,
                    input,
                    worker,
                    &page,
                    reply,
                    device,
                    events,
                    indicator,
                    release_while_idle,
                )?;
                Ok(true)
            }
            // The second tap's capture ends unheard, as a short tap's does,
            // and the task list shows or hides.
            HotkeyAction::DoubleTap => {
                self.question = None;
                handle(action, context, events)?;
                toggle_tasks();
                feedback::play(Tone::OnboardingSuccess);
                Ok(true)
            }
            _ => {
                self.question = None;
                handle(action, context, events)
            }
        }
    }

    /// The press became real (held past the hold threshold with no other
    /// key): a Here question starts reading the desktop now, and its lane
    /// gets ready to answer.
    pub(super) fn intend(&mut self) {
        if let Some(question) = self.question.as_mut()
            && question.intend()
        {
            crate::assistant::service::prewarm(question.lane());
        }
    }

    /// Whether the tap keeps Escape from the app in front for the assistant
    /// now, remembered for the Escape that arrives next.
    pub(super) fn arm_escape(&mut self, bound: bool, free: bool) -> bool {
        self.escape = super::guards::assistant_claims_escape(bound, free, || {
            quick_question_running() || crate::assistant::speech::speaking()
        });
        self.escape
    }

    /// A key for the assistant: any key ends conversation mode (closing its
    /// capture before a shortcut can start another); the Stop all tasks
    /// shortcut stops every task and the speech; Escape while the assistant
    /// took it stops the Here tasks and the speech, never a Background task
    /// (each has its own stop).
    pub(super) fn key(
        &mut self,
        event: InputEvent,
        input: &DictationAudio,
        indicator: DictationSink<'_>,
    ) {
        self.talk.key(event, input, indicator);
        let stop_all = crate::app_settings::runtime_hotkeys().stop_all;
        if stop_all_press(event, stop_all, &mut self.stop_held) {
            crate::assistant::service::stop_all();
            crate::assistant::speech::stop();
            feedback::play(Tone::Cancel);
            return;
        }
        if !self.escape || !event.is_escape_down() {
            return;
        }
        for conversation in here_tasks(&crate::assistant::tasks()) {
            crate::assistant::cancel(&conversation);
        }
        crate::assistant::speech::stop();
    }

    /// Microphone audio for conversation mode, which hears only its own
    /// capture.
    pub(super) fn hear(&mut self, samples: &[f32], rate: u32, indicator: DictationSink<'_>) {
        self.talk.hear(samples, rate, indicator);
    }

    /// Conversation mode's turn in the listener's loop: after a Here answer
    /// it listens for a follow-up, and with barge-in hears one over the
    /// answer (`recognition::talk`). `blocked`: another capture, a command's
    /// dictation, or a pause owns the microphone.
    pub(super) fn converse(
        &mut self,
        blocked: bool,
        input: &DictationAudio,
        worker: &DictationWorker,
        events: &mut EventLog,
        indicator: DictationSink<'_>,
        release_while_idle: bool,
    ) -> Result<()> {
        self.talk.converse(
            blocked,
            input,
            worker,
            events,
            indicator,
            release_while_idle,
        )
    }

    /// Whether conversation mode's capture is open (a microphone loss then
    /// never salvages it as a dictation).
    pub(super) fn conversing(&self) -> bool {
        self.talk.conversing()
    }
}

/// Whether `event` is a press of the Stop all tasks shortcut `binding`
/// (`None` with the assistant off or it unbound). `held` keeps its key while it is
/// down, so the key's repeats are not presses.
fn stop_all_press(
    event: InputEvent,
    binding: Option<RuntimeHotkey>,
    held: &mut Option<u16>,
) -> bool {
    let InputEvent::Key { code, down, flags } = event else {
        return false;
    };
    if !down {
        if *held == Some(code) {
            *held = None;
        }
        return false;
    }
    let pressed = *held != Some(code)
        && binding.is_some_and(|binding| binding.matches_key_press(code, flags));
    if pressed {
        *held = Some(code);
    }
    pressed
}

/// A `hex dictation stop` or `toggle` leaves a question being asked alone:
/// only the Action shortcut's release asks it.
pub(super) fn busy_reply() -> ControlReply {
    ControlReply::error(
        "busy",
        "the Action shortcut is asking the assistant; release it, or cancel with \
         `hex dictation cancel`",
    )
}

/// Ends the question's capture at `at` and hands its clip to transcription
/// for `reply`, as a shortcut dictation's finish does for a paste.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_question(
    at: CaptureInstant,
    input: &DictationAudio,
    worker: &DictationWorker,
    page: &ContextSnapshot,
    reply: PrintReply,
    device: &str,
    events: &mut EventLog,
    indicator: DictationSink<'_>,
    release_while_idle: bool,
) -> Result<()> {
    if input.become_intentional(at)? && release_while_idle {
        feedback::play(Tone::DictationStart);
    }
    // A discarded or silent clip drops the reply unanswered: nothing is asked.
    let finished = super::finish_dictation_with(
        input,
        at,
        POST_RELEASE_TAIL,
        |clip, _| worker.transcribe_for_assistant(clip, reply, page.clone()),
        device,
        events,
        indicator,
    )?;
    if matches!(finished, FinishResult::Discarded) {
        feedback::retract_start();
    }
    super::emit_engine_state(events, false, worker, device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::{Context, Lane, Origin, Request, TaskStatus};
    use crate::control::PrintedTranscript;
    use std::sync::mpsc;
    use std::time::Duration;

    fn read(text: &str) -> ContextHandle {
        let context = Context {
            frontmost_app: Some(text.into()),
            ..Context::default()
        };
        ContextHandle::start(move |reading| reading.quick(context))
    }

    fn nothing() -> ContextHandle {
        ContextHandle::start(|_| {})
    }

    /// A hold asks Here: it continues the focused conversation (the running
    /// Here task, which it steers) with the desktop read at its press. A tap
    /// then a hold starts its own Background task, without one.
    #[test]
    fn a_hold_asks_with_its_lane_settled_at_the_press() {
        let quick = Question::press(false, || read("Mail"));
        assert_eq!(quick.lane(), Lane::Here);
        let quick = quick
            .request("  What's this email about? ", || Some("c1".into()))
            .expect("a quick question");
        assert_eq!(quick.origin, Origin::ActionShortcut);
        assert_eq!(quick.conversation.as_deref(), Some("c1"));
        assert_eq!(quick.text, "What's this email about?");
        assert_eq!(quick.context.frontmost_app.as_deref(), Some("Mail"));

        let task = Question::press(true, || unreachable!("a Background task reads nothing"));
        assert_eq!(task.lane(), Lane::Background);
        let task = task
            .request("File these receipts", || {
                unreachable!("a Background task continues none")
            })
            .expect("a task");
        assert_eq!(task.origin, Origin::Background);
        assert_eq!(task.conversation, None);
        assert_eq!(task.context.frontmost_app, None);
    }

    /// A press reads nothing until the hold is real, and a press discarded
    /// before then (a ⌘C chord) never reads at all.
    #[test]
    fn the_desktop_is_read_only_once_the_hold_is_real() {
        let (started, reads) = std::sync::mpsc::channel();
        let counted = move |sender: std::sync::mpsc::Sender<()>| {
            move || {
                let _ = sender.send(());
                read("Mail")
            }
        };
        drop(Question::press(false, counted(started.clone())));
        assert!(reads.try_recv().is_err(), "a chord read the desktop");

        let mut question = Question::press(false, counted(started));
        assert!(reads.try_recv().is_err(), "the press read the desktop");
        // Only the first says the hold just became real (prewarm once).
        assert!(question.intend());
        assert!(!question.intend());
        assert_eq!(reads.try_iter().count(), 1);
        let asked = question
            .request("What is this?", || None)
            .expect("a question");
        assert_eq!(asked.context.frontmost_app.as_deref(), Some("Mail"));
    }

    /// Silence asks nothing.
    #[test]
    fn no_words_ask_nothing() {
        assert!(
            Question::press(false, nothing)
                .request("  \n", || None)
                .is_none()
        );
    }

    /// The transcript reaches `submit` from the question's own thread; a
    /// dropped reply (a short tap, a discard, a cancel) and an empty
    /// transcript submit nothing.
    #[test]
    fn the_transcript_submits_and_a_dropped_reply_asks_nothing() {
        let (submitted, asked) = mpsc::channel();
        let submit = move |sender: mpsc::Sender<Request>| {
            move |request| {
                let _ = sender.send(request);
            }
        };
        let reply = Question::press(false, nothing).ask(|| None, submit(submitted.clone()));
        reply.send(PrintedTranscript::Text("Summarize this page".into()));
        let request = asked
            .recv_timeout(Duration::from_secs(5))
            .expect("the question is asked");
        assert_eq!(request.origin, Origin::ActionShortcut);
        assert_eq!(request.text, "Summarize this page");

        drop(Question::press(true, nothing).ask(|| None, submit(submitted.clone())));
        Question::press(false, nothing)
            .ask(|| None, submit(submitted.clone()))
            .send(PrintedTranscript::NoSpeech);
        Question::press(true, nothing)
            .ask(|| None, submit(submitted))
            .send(PrintedTranscript::Text("   ".into()));
        assert!(asked.recv_timeout(Duration::from_millis(200)).is_err());
    }

    /// "In the background …" sends a Here question to the Background lane
    /// without those words, in German or English, whatever the case or the
    /// punctuation around them.
    #[test]
    fn spoken_openings_route_to_the_background() {
        for (said, rest) in [
            (
                "Im Hintergrund räum den Downloads-Ordner auf",
                "räum den Downloads-Ordner auf",
            ),
            ("im hintergrund, räum auf.", "räum auf."),
            ("IM HINTERGRUND: Mails sortieren", "Mails sortieren"),
            ("  „Im Hintergrund“ – buch den Flug", "buch den Flug"),
            (
                "In the background, file these receipts",
                "file these receipts",
            ),
            ("in the Background: check my calendar", "check my calendar"),
            (
                "In the background... summarise the PDF",
                "summarise the PDF",
            ),
            ("Nebenbei: schreib Anna zurück", "schreib Anna zurück"),
            ("nebenbei bestell Kaffee", "bestell Kaffee"),
            ("Parallel, update the spreadsheet", "update the spreadsheet"),
            (
                "parallel lad die Rechnungen herunter",
                "lad die Rechnungen herunter",
            ),
            ("Hintergrund: Downloads aufräumen", "Downloads aufräumen"),
            ("hintergrund : Downloads aufräumen", "Downloads aufräumen"),
            ("Im Hintergrund", ""),
            ("In the background.", ""),
        ] {
            assert_eq!(spoken_background(said), Some(rest), "{said:?}");
        }
    }

    /// Only the whole words open it: a word that merely starts like one, the
    /// words later in the sentence, or "Hintergrund" without its colon ask
    /// about the window in front.
    #[test]
    fn words_that_merely_contain_an_opening_stay_here() {
        for said in [
            "Parallelogramm zeichnen",
            "Parallels Desktop öffnen",
            "Hintergrundbild ändern",
            "Hintergrund ändern",
            "Hintergrund-Prozesse anzeigen",
            "Nebenbeigesagt, was steht hier?",
            "Im Hintergrundbild ist was?",
            "Imhintergrund aufräumen",
            "In the backgrounds folder, what's new?",
            "In the back, what's that?",
            "In thebackground",
            "What runs in the background?",
            "Mach das parallel",
            "",
            "   ",
        ] {
            assert_eq!(spoken_background(said), None, "{said:?}");
        }
    }

    /// A Here question that opens with "im Hintergrund" becomes a new
    /// Background task: no conversation, no screenshot, and the read its
    /// hold started is dropped. Nothing after the opening asks nothing.
    #[test]
    fn a_spoken_opening_asks_a_background_task() {
        let mut question = Question::press(false, || read("Mail"));
        question.intend();
        let task = question
            .request("Im Hintergrund, räum die Downloads auf", || {
                unreachable!("a Background task continues none")
            })
            .expect("a task");
        assert_eq!(task.origin, Origin::Background);
        assert_eq!(task.conversation, None);
        assert_eq!(task.text, "räum die Downloads auf");
        assert!(task.context.screenshot.is_none());
        assert_eq!(task.context.frontmost_app, None);

        assert!(
            Question::press(false, nothing)
                .request("In the background.", || None)
                .is_none()
        );
        // After a tap the words are the task's, opening and all.
        let task = Question::press(true, nothing)
            .request("Parallelogramm zeichnen", || None)
            .expect("a task");
        assert_eq!(task.text, "Parallelogramm zeichnen");
    }

    fn task(conversation: &str, origin: Origin) -> TaskStatus {
        TaskStatus {
            conversation: conversation.into(),
            origin,
            title: conversation.into(),
            state: crate::assistant::TaskState::Running,
            step: None,
            windows: Vec::new(),
            colour: 0,
        }
    }

    /// Escape stops the Here tasks only: never a Background or Chats task.
    #[test]
    fn escape_stops_only_here_tasks() {
        let tasks = [
            task("background", Origin::Background),
            task("here", Origin::ActionShortcut),
            task("chat", Origin::Chat),
        ];
        assert_eq!(here_tasks(&tasks), ["here"]);
        assert!(here_tasks(&tasks[..1]).is_empty());
        assert!(here_tasks(&[]).is_empty());
    }

    const ESCAPE: u16 = 53;

    fn key(code: u16, down: bool, flags: u64) -> InputEvent {
        InputEvent::Key { code, down, flags }
    }

    /// ⌃⌥Esc stops everything once per press, its repeats stop nothing, and
    /// Escape alone, ⌘⌥Esc (Force Quit), or the shortcut unbound or the
    /// assistant off (`None`) is no press.
    #[test]
    fn the_stop_all_shortcut_presses_once_and_only_while_bound() {
        use crate::app_settings::{COMMAND_KEY_MASK, CONTROL_KEY_MASK, OPTION_KEY_MASK};
        let binding = Some(crate::app_settings::HotkeyBinding::stop_all_default().runtime());
        let chord = CONTROL_KEY_MASK | OPTION_KEY_MASK;
        let mut held = None;
        assert!(stop_all_press(key(ESCAPE, true, chord), binding, &mut held));
        assert!(!stop_all_press(
            key(ESCAPE, true, chord),
            binding,
            &mut held
        ));
        assert!(!stop_all_press(
            key(ESCAPE, false, chord),
            binding,
            &mut held
        ));
        assert!(stop_all_press(key(ESCAPE, true, chord), binding, &mut held));
        assert!(!stop_all_press(key(ESCAPE, false, 0), binding, &mut held));

        assert!(!stop_all_press(key(ESCAPE, true, 0), binding, &mut held));
        assert!(!stop_all_press(key(ESCAPE, false, 0), binding, &mut held));
        let force_quit = COMMAND_KEY_MASK | OPTION_KEY_MASK;
        assert!(!stop_all_press(
            key(ESCAPE, true, force_quit),
            binding,
            &mut held
        ));
        assert!(!stop_all_press(key(12, true, chord), binding, &mut held));
        assert!(!stop_all_press(
            InputEvent::Flags(chord),
            binding,
            &mut held
        ));

        let mut held = None;
        assert!(!stop_all_press(key(ESCAPE, true, chord), None, &mut held));
        assert_eq!(held, None);
    }

    /// A double tap calls what the overlays set; until then it does nothing.
    #[test]
    fn a_double_tap_calls_the_task_list_toggle() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static TOGGLED: AtomicUsize = AtomicUsize::new(0);
        toggle_tasks();
        set_toggle_tasks(|| {
            TOGGLED.fetch_add(1, Ordering::SeqCst);
        });
        toggle_tasks();
        toggle_tasks();
        assert_eq!(TOGGLED.load(Ordering::SeqCst), 2);
    }

    /// Before the setup is finished only a hold opens it: a tap or a chord
    /// released before the hold threshold never does, and the hook runs
    /// once it is set.
    #[test]
    fn only_a_hold_opens_the_setup() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let at = |ms: u64| CaptureInstant::ZERO + Duration::from_millis(ms);
        assert!(held(at(1_000), at(1_300)));
        assert!(held(at(1_000), at(4_000)));
        assert!(!held(at(1_000), at(1_080)));
        assert!(!held(at(1_000), at(1_299)));
        assert!(!held(at(1_000), at(900)));

        static OPENED: AtomicUsize = AtomicUsize::new(0);
        open_setup();
        set_open_setup(|| {
            OPENED.fetch_add(1, Ordering::SeqCst);
        });
        open_setup();
        assert_eq!(OPENED.load(Ordering::SeqCst), 1);
    }
}
