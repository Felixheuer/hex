//! One task's run, on its own thread, to its end: the backend run, and what
//! may come between runs of the same task (waiting out a usage limit, a
//! job's question and the person's answer), then the end (its final event,
//! its windows and browser released, its outcome recorded) or its handover
//! (steered, or escalated from a Here or Chat request into a Background
//! job).

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use crate::assistant::autonomy::{self, Authorized, Autonomy};

use super::{
    BLIND_RETRIES, Context, ConversationSummary, Ended, Event, Job, JobKind, Lane, Origin, Outcome,
    Request, Runner, TaskState, TaskStatus, backoff, context,
};

/// How often a quick turn's thinking line looks whether it is due.
const THINKING_POLL: Duration = Duration::from_millis(250);
/// Past this, a turn still silent says nothing more: whatever kept the
/// line back (another reading, the person talking) took the moment.
const THINKING_LINE_UNTIL: Duration = Duration::from_secs(8);

impl Runner {
    pub(super) fn run_task(self: &Arc<Self>, key: u64) {
        let (_, settings) = (self.hooks.settings)();
        let now = self.now();
        let Some((request, control, kind, new_conversation, voice, said)) = ({
            let mut state = self.state();
            state
                .tasks
                .iter_mut()
                .find(|task| task.key == key)
                .map(|task| {
                    task.speak = task.lane() == Lane::Here && settings.speak_answers;
                    task.limit_minutes = limit_minutes(task.lane(), &settings);
                    task.deadline_ms = (task.limit_minutes > 0)
                        .then(|| now + u64::from(task.limit_minutes) * 60_000);
                    (
                        task.request.clone(),
                        Arc::clone(&task.control),
                        task.kind.clone(),
                        task.new_conversation,
                        task.voice,
                        task.said.clone(),
                    )
                })
        }) else {
            return;
        };
        let conversation = request.conversation.clone().unwrap_or_default();
        let lane = request.origin.lane();
        // An answer of it cut by a stop is over: this one is read.
        self.voice.started(&conversation);
        self.emit(&Event::Started {
            conversation: conversation.clone(),
            origin: request.origin,
            question: request.text.clone(),
        });
        // Asked by voice and still silent after a moment: one short line.
        if lane == Lane::Here && voice && settings.speak_answers {
            self.say_thinking_line(key, &conversation, &request.text);
        }
        // A helper works in a folder inside its lead's (`agent::team`).
        let parent = self
            .state()
            .tasks
            .iter()
            .find(|task| task.key == key)
            .and_then(|task| task.parent.clone());
        let inside = parent.as_ref().and_then(|lead| {
            self.state()
                .tasks
                .iter()
                .find(|task| &task.status.conversation == lead)
                .and_then(|task| task.workspace.clone())
                .or_else(|| (self.hooks.workspace_of)(lead))
        });
        // The project it runs in (`projects`): its conversation's (a
        // helper's lead's), else the active one for a new conversation. A
        // job's folder is made inside it.
        let project = self.project_for(
            &conversation,
            parent.as_deref(),
            new_conversation,
            &request.text,
        );
        let inside = inside.or_else(|| project.as_ref().map(|project| project.jobs.clone()));
        match &project {
            Some(project) => self.emit(&Event::InProject {
                conversation: conversation.clone(),
                name: project.name.clone(),
            }),
            None if new_conversation && parent.is_none() => {
                self.suggest_project(&conversation, &request.text);
            }
            None => {}
        }
        let project_id = project.as_ref().map(|project| project.id.clone());
        let workspace = match &kind {
            JobKind::Job { title } => {
                let workspace = (self.hooks.make_workspace)(inside.as_deref(), title);
                if workspace.is_none() {
                    tracing::warn!("couldn't make the job's workspace; it runs without one");
                }
                workspace
            }
            _ => (self.hooks.workspace_of)(&conversation),
        };
        if let Some(task) = self.state().tasks.iter_mut().find(|task| task.key == key) {
            task.workspace = workspace.clone();
        }
        // A job gets the stream without the turn's tools for it (`tools::Role`).
        let turn = matches!(kind, JobKind::Ask)
            && request.origin != crate::assistant::Origin::Background
            && workspace.is_none();
        let digest = (new_conversation && matches!(kind, JobKind::Ask | JobKind::Job { .. }))
            .then(|| self.digest(&conversation, turn))
            .flatten();
        // What the person's own words allow (never what a task reads, nor
        // a task an agent wrote: `Said`).
        let mut authorized = if new_conversation {
            Authorized::default()
        } else {
            (self.hooks.authorized)(&conversation)
        };
        if settings.autonomy != Autonomy::Ask
            && let Some(said) = &said
        {
            autonomy::merge(&mut authorized, autonomy::from_request(said));
        }
        (self.hooks.authorize)(&conversation, &authorized);
        let memory = settings
            .memory
            .then(|| crate::assistant::memory::for_prompt(&(self.hooks.memory)()))
            .flatten();
        // What the graph knows about what the words name goes with the
        // message: the system prompt stays the one started ahead.
        let recall = settings
            .memory
            .then(|| (self.hooks.recall)(&request.text, project_id.as_deref()))
            .flatten();
        let notify = |event: &Event| {
            // Away from the Mac: questions and results reach the phone.
            if parent.is_none() {
                crate::assistant::phone::on_event(event, &request.text);
            }
            // A helper is quiet: its lead answers for the team.
            if parent.is_none()
                && crate::assistant::notify::wanted(
                    settings.notifications,
                    lane == Lane::Background,
                    request.origin == Origin::Chat,
                    crate::assistant::notify::hex_in_front(),
                )
                && let Some(note) = crate::assistant::notify::for_event(event, &request.text)
            {
                (self.hooks.notify)(&note);
            }
        };
        // A job's browsers start now, while its agent does, so its first
        // page opens warm (a helper's started when its lead made it).
        let starts_job = matches!(kind, JobKind::Job { .. })
            || (kind == JobKind::Ask && request.origin == Origin::Background);
        if starts_job && lane == Lane::Background && settings.background_browser && parent.is_none()
        {
            (self.hooks.browser_ahead)(&conversation, &request.text);
        }
        let mut job = Job {
            conversation: conversation.clone(),
            request: request.clone(),
            settings: settings.clone(),
            kind,
            digest,
            workspace: workspace.clone(),
            authorized,
            memory,
            recall,
            project,
            helps: parent.as_ref().and_then(|lead| {
                self.state()
                    .tasks
                    .iter()
                    .find(|task| &task.status.conversation == lead)
                    .map(|task| task.status.title.clone())
            }),
        };
        let mut attempts = 0;
        let outcome = loop {
            let outcome = if control.cancelled() {
                Outcome::Cancelled
            } else {
                std::panic::catch_unwind(AssertUnwindSafe(|| {
                    (self.hooks.execute)(&job, &control, &mut |event| self.emit(&event))
                }))
                .unwrap_or_else(|_| {
                    tracing::error!("an assistant task panicked");
                    Outcome::Failed("The task stopped unexpectedly.".into())
                })
            };
            match outcome {
                Outcome::Limited { reason, until_ms } => {
                    attempts += 1;
                    // A job keeps trying for its whole budget (backoff tops
                    // out at half an hour); a question gives up sooner.
                    let retries = match lane {
                        Lane::Background => (settings.job_budget_minutes / 30).max(BLIND_RETRIES),
                        Lane::Here => BLIND_RETRIES,
                    };
                    if until_ms.is_none() && attempts > retries {
                        break Outcome::Failed(format!(
                            "{reason} (still limited after {attempts} tries)"
                        ));
                    }
                    let until = until_ms.unwrap_or_else(|| self.now() + backoff(attempts));
                    self.wait_out(key, &conversation, &control, until, reason);
                    if control.cancelled() {
                        break Outcome::Cancelled;
                    }
                    job.kind = JobKind::AfterLimit;
                }
                Outcome::Question(question) => {
                    // Read aloud when it becomes the inbox's active one.
                    notify(&Event::Question {
                        conversation: conversation.clone(),
                        text: question.clone(),
                    });
                    let Some(answer) = self.ask(key, &conversation, &control, question.clone())
                    else {
                        break Outcome::Cancelled;
                    };
                    // The answer is the person's own words too.
                    if settings.autonomy != Autonomy::Ask {
                        autonomy::merge(&mut job.authorized, autonomy::from_request(&answer));
                        (self.hooks.authorize)(&conversation, &job.authorized);
                    }
                    job.kind = JobKind::Answer { question };
                    job.request.text = answer;
                    job.request.context = Context::default();
                }
                Outcome::Confirm { category, question } => {
                    notify(&Event::Question {
                        conversation: conversation.clone(),
                        text: question.clone(),
                    });
                    let Some(answer) = self.ask(key, &conversation, &control, question.clone())
                    else {
                        break Outcome::Cancelled;
                    };
                    let allowed = autonomy::is_yes(&answer);
                    if allowed {
                        job.authorized = (self.hooks.authorized)(&conversation);
                        // For the batch it asked about, not the rest of the
                        // job (`autonomy::CONFIRMED_FOR_MS`).
                        job.authorized.confirm(category, self.now());
                        (self.hooks.authorize)(&conversation, &job.authorized);
                    }
                    job.kind = JobKind::Confirm { question, allowed };
                    job.request.text = answer;
                    job.request.context = Context::default();
                }
                other => break other,
            }
            job.digest = None;
        };
        let outcome = if control.steered() {
            Outcome::Steered
        } else if let (Some(reason), Outcome::Cancelled) = (control.failure(), &outcome) {
            Outcome::Failed(reason)
        } else if control.cancelled() {
            Outcome::Cancelled
        } else {
            outcome
        };
        // Steered, but the run it was steered to is gone (stopped while this
        // one wound down): nothing goes on, so it ends as stopped, or the
        // conversation would stay running and come back after a restart.
        let outcome = if outcome == Outcome::Steered
            && !self
                .state()
                .tasks
                .iter()
                .any(|task| task.key != key && task.status.conversation == conversation)
        {
            Outcome::Cancelled
        } else {
            outcome
        };
        let handed_on = matches!(outcome, Outcome::Steered | Outcome::Escalate { .. });
        let now = self.now();
        let (paused, hands_off, orphaned) = {
            let mut state = self.state();
            let paused = state
                .tasks
                .iter()
                .find(|task| task.key == key)
                .is_some_and(|task| task.status.state == TaskState::Paused);
            state.tasks.retain(|task| task.key != key);
            // A lead that ended (not one handed on in the same
            // conversation) leaves no helpers working for nobody.
            let orphaned: Vec<u64> = if handed_on {
                Vec::new()
            } else {
                state
                    .tasks
                    .iter()
                    .filter(|task| task.parent.as_deref() == Some(conversation.as_str()))
                    .map(|task| task.key)
                    .collect()
            };
            if lane == Lane::Here && matches!(outcome, Outcome::Answer(_)) {
                state.last_quick = Some((conversation.clone(), now));
            }
            let hands_off = (state.hands_off.as_deref() == Some(conversation.as_str())
                && outcome != Outcome::Steered)
                .then(|| state.hands_off.take())
                .flatten();
            (paused, hands_off, orphaned)
        };
        for helper in orphaned {
            self.cancel_task(helper);
        }
        if let Some(conversation) = hands_off {
            self.emit(&Event::HandsOff {
                conversation,
                on: false,
            });
        }
        // A new instruction for a task the person had taken over hands it
        // back: the run it goes on in isn't held by the old pause.
        if paused {
            (self.hooks.pause)(&conversation, false);
        }
        if outcome != Outcome::Steered {
            // The lane this run ended in, not the conversation's first
            // origin: an escalated job's browsers close (`browser::close`).
            (self.hooks.release)(&conversation, lane);
        }
        if !handed_on {
            let (ended, message) = match &outcome {
                Outcome::Answer(_) => (Ended::Answered, ""),
                Outcome::Failed(message) => (Ended::Failed, message.as_str()),
                _ => (Ended::Cancelled, "Stopped."),
            };
            (self.hooks.settle)(&conversation, ended, message);
        }
        // Always warm: the next new quick conversation finds its backend up.
        if lane == Lane::Here && matches!(outcome, Outcome::Answer(_)) {
            self.stand_by();
        }
        // What it did, call by call, beside what it made: the folder's
        // list in Chats shows it.
        if !handed_on && let Some(folder) = &workspace {
            crate::assistant::audit::keep_in(&conversation, folder);
            // What its journal says worked or failed in an app, for the
            // next task there (`procedures`).
            if settings.memory {
                let journal =
                    std::fs::read_to_string(folder.join(crate::assistant::agent::journal::JOURNAL))
                        .unwrap_or_default();
                crate::assistant::procedures::learn_from_journal(&journal, project_id.as_deref());
            }
        }
        match outcome {
            Outcome::Answer(text) => {
                if settings.memory {
                    (self.hooks.learn)(&conversation, &request.text, &text);
                }
                // What the person named and the job made goes into the
                // project's soul (`projects::Projects::collect`).
                if let Some(id) = &project_id {
                    self.collect_for_project(id, &request.text, &text, workspace.as_deref());
                }
                let text = with_workspace(text, workspace.as_deref());
                if settings.speak_answers {
                    if lane == Lane::Here {
                        self.voice.finish(self, &conversation, &text);
                    } else if voice {
                        // A job asked by voice: its spoken summary, in
                        // turn after what plays (`voice::Priority`).
                        self.voice.announce(
                            self,
                            super::voice::Priority::Summary,
                            &conversation,
                            &request.text,
                            &spoken_summary(&text),
                        );
                    }
                }
                let event = Event::Answer {
                    conversation,
                    origin: request.origin,
                    text,
                };
                notify(&event);
                self.emit(&event);
            }
            Outcome::Failed(message) => {
                self.voice.drop_answer(&conversation);
                let event = Event::Failed {
                    conversation,
                    message,
                };
                notify(&event);
                self.emit(&event);
            }
            Outcome::Cancelled => {
                self.voice.drop_answer(&conversation);
                self.emit(&Event::Cancelled { conversation });
            }
            Outcome::Escalate {
                title,
                reply,
                more,
                first_task,
            } => {
                // The line said meanwhile ("On it: …") shows where it's
                // heard: the Here bubble holds it until it was read.
                if !reply.is_empty() {
                    self.emit(&Event::AnswerDelta {
                        conversation: conversation.clone(),
                        text: reply.clone(),
                    });
                }
                if voice && settings.speak_answers && !reply.is_empty() {
                    self.voice.finish(self, &conversation, &reply);
                }
                // A request split into several jobs: the conversation goes on
                // with the first one's own part.
                let request = match first_task {
                    Some(task) => Request {
                        text: task,
                        ..request
                    },
                    None => request,
                };
                self.escalate(request, title, new_conversation, voice, said.clone());
                // The other jobs' tasks are the agent's words: they are
                // authorized from the person's request that led to them.
                for (title, text) in more {
                    self.submit_as(
                        Request {
                            conversation: None,
                            text,
                            origin: Origin::Background,
                            context: Context::default(),
                        },
                        JobKind::Job { title },
                        said.clone().map_or(super::Said::Nobody, super::Said::Words),
                    );
                }
            }
            Outcome::Steered
            | Outcome::Limited { .. }
            | Outcome::Question(_)
            | Outcome::Confirm { .. } => {
                self.voice.drop_answer(&conversation);
            }
        }
        self.refresh_mode();
        self.pump();
    }

    /// Says the thinking line (`voice::thinking_line`) for the quick turn
    /// `key` once it has been silent for `voice::THINKING_LINE_AFTER`, on a
    /// thread of its own: never once a word of its answer streamed, it
    /// asked, ended, or was stopped, nor over anything read or said; given
    /// up after [`THINKING_LINE_UNTIL`].
    fn say_thinking_line(self: &Arc<Self>, key: u64, conversation: &str, said: &str) {
        let runner = Arc::clone(self);
        let reading = format!("thinking:{conversation}");
        let said = said.to_owned();
        let since = self.now();
        let watch = move || {
            loop {
                (runner.hooks.sleep)(THINKING_POLL);
                let heard = {
                    let state = runner.state();
                    match state.tasks.iter().find(|task| task.key == key) {
                        Some(task)
                            if !task.control.cancelled()
                                && task.status.state == TaskState::Running =>
                        {
                            task.heard
                        }
                        _ => return,
                    }
                };
                let now = runner.now();
                if heard || now.saturating_sub(since) > THINKING_LINE_UNTIL.as_millis() as u64 {
                    return;
                }
                let busy = runner.voice.reading()
                    || (runner.hooks.speaking)()
                    || (runner.hooks.person_talks)();
                if super::voice::thinking_line_due(since, now, heard, busy) {
                    // Its own reading: the answer's first words replace it
                    // if they come before it was handed to the voice.
                    runner
                        .voice
                        .finish(&runner, &reading, super::voice::thinking_line(&said));
                    return;
                }
            }
        };
        let _ = std::thread::Builder::new()
            .name("assistant-thinking-line".into())
            .spawn(watch);
    }

    /// Continues `request` (a Here or Chat request that turned out to be a
    /// job) as a job in the same conversation: `Event::Escalated`, then a
    /// Background (or Chat) task that plans first.
    pub(super) fn escalate(
        self: &Arc<Self>,
        request: Request,
        title: String,
        new_conversation: bool,
        voice: bool,
        said: Option<String>,
    ) {
        let conversation = request.conversation.clone().unwrap_or_default();
        let origin = match request.origin {
            Origin::ActionShortcut => Origin::Background,
            other => other,
        };
        let request = Request {
            origin,
            context: Context {
                screenshot: None,
                window: None,
                ..request.context
            },
            ..request
        };
        self.emit(&Event::Escalated {
            conversation: conversation.clone(),
        });
        {
            let mut state = self.state();
            self.push_task(
                &mut state,
                request,
                JobKind::Job { title },
                None,
                new_conversation,
                voice,
                said,
            );
        }
        self.refresh_mode();
        self.pump();
    }

    /// Asks the person `question` for task `key` and waits for the answer
    /// (`TaskState::NeedsInput`); `None` when the task was stopped instead.
    fn ask(
        self: &Arc<Self>,
        key: u64,
        conversation: &str,
        control: &super::Control,
        question: String,
    ) -> Option<String> {
        let now = self.now();
        if let Some(task) = self.state().tasks.iter_mut().find(|task| task.key == key) {
            task.status.state = TaskState::NeedsInput;
            task.question = Some(question.clone());
            task.asked_ms = Some(now);
            task.reply = None;
            task.held_since.get_or_insert(now);
        }
        self.emit(&Event::Question {
            conversation: conversation.to_owned(),
            text: question,
        });
        self.questions_changed();
        loop {
            if control.cancelled() {
                if let Some(task) = self.state().tasks.iter_mut().find(|task| task.key == key) {
                    task.question = None;
                    task.asked_ms = None;
                }
                self.questions_changed();
                return None;
            }
            let answer = {
                let now = self.now();
                let mut state = self.state();
                let task = state.tasks.iter_mut().find(|task| task.key == key)?;
                task.reply.take().inspect(|_| {
                    task.status.state = TaskState::Running;
                    task.question = None;
                    task.asked_ms = None;
                    super::monitor::resume_clock(task, now);
                })
            };
            if answer.is_some() {
                self.questions_changed();
                return answer;
            }
            (self.hooks.sleep)(Duration::from_millis(100));
        }
    }

    /// What else is going on, for a new conversation's first prompt.
    pub(super) fn digest(&self, conversation: &str, turn: bool) -> Option<String> {
        let running: Vec<TaskStatus> = self
            .state()
            .tasks
            .iter()
            .filter(|task| task.status.conversation != conversation)
            .map(|task| task.status.clone())
            .collect();
        let recent: Vec<ConversationSummary> = (self.hooks.recent)()
            .into_iter()
            .filter(|summary| summary.id != conversation)
            .collect();
        context::digest(&running, &recent, self.now(), turn)
    }
}

/// A job's answer names its workspace; Agent-Desk adds it when the agent didn't.
fn with_workspace(text: String, workspace: Option<&std::path::Path>) -> String {
    match workspace {
        Some(workspace) if !text.contains(&*workspace.to_string_lossy()) => {
            format!("{}\n\nFiles: {}", text.trim_end(), workspace.display())
        }
        _ => text,
    }
}

/// How long a task may run: a Background job its budget (up to seven
/// days), a question or chat the shorter task limit; 0 never stops it.
pub(super) fn limit_minutes(lane: Lane, settings: &crate::app_settings::AssistantSettings) -> u32 {
    match lane {
        Lane::Background => settings.job_budget_minutes,
        Lane::Here => settings.task_timeout_minutes,
    }
}

/// A time limit as people say it: "45 minutes", "6 hours", "7 days".
pub(super) fn limit_words(minutes: u32) -> String {
    match minutes {
        m if m >= 1440 && m % 1440 == 0 => plural(m / 1440, "day"),
        m if m >= 60 && m % 60 == 0 => plural(m / 60, "hour"),
        m => plural(m, "minute"),
    }
}

fn plural(n: u32, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// Sentences of a spoken summary: enough to tell a research result, short
/// enough to listen to.
const SPOKEN_SENTENCES: usize = 4;

/// What a job asked by voice reads aloud: its answer's first paragraph (the
/// summary the job rules ask for), at most [`SPOKEN_SENTENCES`] sentences.
/// Lists, files, and sources below it stay in Chats.
fn spoken_summary(text: &str) -> String {
    let text = text.trim();
    let paragraph = text.split("\n\n").next().unwrap_or(text).trim();
    let mut end = paragraph.len();
    let mut sentences = 0;
    for (index, c) in paragraph.char_indices() {
        let next = paragraph[index + c.len_utf8()..].chars().next();
        if matches!(c, '.' | '!' | '?') && next.is_none_or(char::is_whitespace) && index > 8 {
            sentences += 1;
            if sentences == SPOKEN_SENTENCES {
                end = index + c.len_utf8();
                break;
            }
        }
    }
    paragraph[..end].replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_answer_names_its_workspace_and_speaks_its_first_sentence() {
        let path = std::path::Path::new("/Users/p/Documents/Agent-Desk/2026-11-12 Job");
        assert_eq!(
            with_workspace("Done.".into(), Some(path)),
            "Done.\n\nFiles: /Users/p/Documents/Agent-Desk/2026-11-12 Job"
        );
        let named = format!("Saved in {}.", path.display());
        assert_eq!(with_workspace(named.clone(), Some(path)), named);
        assert_eq!(
            spoken_summary("Found 3 suppliers. The sheet is in the workspace.\n\n- A\n- B"),
            "Found 3 suppliers. The sheet is in the workspace."
        );
        assert_eq!(spoken_summary("Done"), "Done");
    }

    #[test]
    fn jobs_get_their_budget_and_questions_the_task_limit() {
        let settings = crate::app_settings::AssistantSettings::default();
        assert_eq!(limit_minutes(Lane::Background, &settings), 7 * 24 * 60);
        assert_eq!(limit_minutes(Lane::Here, &settings), 30);
        assert_eq!(limit_words(7 * 24 * 60), "7 days");
        assert_eq!(limit_words(60), "1 hour");
        assert_eq!(limit_words(90), "90 minutes");
        assert_eq!(limit_words(30), "30 minutes");
        assert_eq!(
            spoken_summary(
                "Eins ist gut. Zwei ist gut. Drei ist gut. Vier ist gut. Fünf ist zu viel."
            ),
            "Eins ist gut. Zwei ist gut. Drei ist gut. Vier ist gut."
        );
    }
}
