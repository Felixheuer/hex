//! Reading a Here answer aloud as it streams: each finished sentence is
//! queued and spoken in turn while the rest is still being written, so the
//! first words come out long before the whole answer is there.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::Runner;
use crate::assistant::lines;

/// A sentence shorter than this waits for the next one ("Ja." alone sounds
/// clipped).
const MIN_SENTENCE: usize = 12;

#[derive(Default)]
pub(super) struct Voice {
    state: Mutex<State>,
    /// Lines waiting their turn (`announce`), kept apart from the answer
    /// being read so a new answer doesn't drop them.
    pending: Mutex<Pending>,
}

/// What is read first when several lines wait: the answer to the person's
/// own question, then the question they should answer, then what finished.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Priority {
    Answer,
    Question,
    Summary,
    #[allow(dead_code, reason = "for notices that may wait behind everything")]
    Misc,
}

/// A line waiting to be read aloud.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Announcement {
    pub priority: Priority,
    pub conversation: String,
    /// The task's title, for a collapsed summary.
    pub title: String,
    pub text: String,
}

#[derive(Default)]
struct Pending {
    items: Vec<Announcement>,
    /// The thread reading them runs.
    pumping: bool,
}

/// More finished jobs than this waiting at once are told in one line.
const COLLAPSE_OVER: usize = 2;

/// How often the waiting lines check whether they may be read.
const PUMP_EVERY: Duration = Duration::from_millis(200);

/// A quick turn asked by voice that is still silent this long says one
/// short line ("On it.", „Mach ich.“), so the person knows it heard them;
/// one that answers sooner says nothing extra (`docs/ux/redesign.md` §2).
pub(super) const THINKING_LINE_AFTER: Duration = Duration::from_secs(2);

/// The thinking line, in the language of what was said.
pub(super) fn thinking_line(said: &str) -> &'static str {
    lines::tr("On it.", said)
}

/// Whether the thinking line is due: [`THINKING_LINE_AFTER`] passed since
/// `since_ms`, the turn still runs without a word of its answer, and nothing
/// else is read or said.
pub(super) fn thinking_line_due(since_ms: u64, now_ms: u64, heard: bool, busy: bool) -> bool {
    !heard && !busy && now_ms.saturating_sub(since_ms) >= THINKING_LINE_AFTER.as_millis() as u64
}

/// How long a hold counts as one at most, should its end never arrive.
const LISTENING_AT_MOST_MS: u64 = 30_000;

static LISTENING_UNTIL: AtomicU64 = AtomicU64::new(0);
/// Until when announcements wait after the person asked for quiet.
static QUIET_UNTIL: AtomicU64 = AtomicU64::new(0);

/// Announcements wait for `quiet` from `now_ms` (Escape, „Stopp“).
pub(super) fn quiet_for(quiet: Duration, now_ms: u64) {
    QUIET_UNTIL.store(now_ms + quiet.as_millis() as u64, Ordering::Release);
}

fn quiet(now_ms: u64) -> bool {
    now_ms < QUIET_UNTIL.load(Ordering::Acquire)
}
static SPOKEN: AtomicBool = AtomicBool::new(true);
static ANNOUNCED: AtomicU64 = AtomicU64::new(0);

/// The person holds the shortcut or dictates (`on`), or stopped.
pub(super) fn set_listening(on: bool, now_ms: u64) {
    let until = if on { now_ms + LISTENING_AT_MOST_MS } else { 0 };
    LISTENING_UNTIL.store(until, Ordering::Release);
}

fn listening(now_ms: u64) -> bool {
    now_ms < LISTENING_UNTIL.load(Ordering::Acquire)
}

/// Whether jobs' lines wait now: the person speaks (a hold or a
/// dictation, [`set_listening`]) or asked for quiet ([`quiet_for`]). The
/// runner asks through its `announcements_wait` hook.
pub(super) fn announcements_wait() -> bool {
    let now = super::store::now_ms();
    listening(now) || quiet(now)
}

/// Whether questions and summaries are read aloud, or only shown.
pub(super) fn set_spoken_announcements(on: bool) {
    SPOKEN.store(on, Ordering::Release);
}

/// Whether questions and summaries are read aloud now.
pub(super) fn spoken_announcements() -> bool {
    SPOKEN.load(Ordering::Acquire)
}

/// The next line to read from `items`, taken out of it: the most urgent,
/// oldest first; more than [`COLLAPSE_OVER`] finished jobs become one line
/// ("3 Aufträge sind fertig: A, B und C.").
pub(super) fn next_announcement(items: &mut Vec<Announcement>) -> Option<Announcement> {
    let priority = items.iter().map(|item| item.priority).min()?;
    if priority == Priority::Summary {
        let summaries = items
            .iter()
            .filter(|item| item.priority == Priority::Summary)
            .count();
        if summaries > COLLAPSE_OVER {
            let (done, rest): (Vec<_>, Vec<_>) = std::mem::take(items)
                .into_iter()
                .partition(|item| item.priority == Priority::Summary);
            *items = rest;
            return Some(collapsed(&done));
        }
    }
    let index = items.iter().position(|item| item.priority == priority)?;
    Some(items.remove(index))
}

/// One line for several finished jobs.
fn collapsed(done: &[Announcement]) -> Announcement {
    let joined: String = done
        .iter()
        .map(|item| item.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let language = lines::language(&joined);
    let titles: Vec<&str> = done.iter().map(|item| item.title.as_str()).collect();
    let (last, first) = titles.split_last().unwrap_or((&"", &[]));
    let names = if first.is_empty() {
        (*last).to_owned()
    } else {
        lines::tr_named_in(
            language,
            "{first} and {last}",
            &[("first", &first.join(", ")), ("last", last)],
        )
    };
    let text = lines::tr_named_in(
        language,
        "{count} jobs are done: {names}.",
        &[("count", &done.len().to_string()), ("names", &names)],
    );
    Announcement {
        priority: Priority::Summary,
        conversation: String::new(),
        title: String::new(),
        text,
    }
}

#[derive(Default)]
struct State {
    /// The conversation being read; others' text is ignored.
    conversation: Option<String>,
    /// Text not yet spoken.
    buffer: String,
    /// Sentences waiting their turn.
    queue: VecDeque<String>,
    /// Something of this answer was queued.
    started: bool,
    /// The whole answer arrived (`finish`): no more of it will stream.
    finished: bool,
    /// The voice for this answer, chosen from its first sentence.
    voice: Option<Option<String>>,
    /// The thread speaking the queue is running.
    speaking: bool,
    /// Bumped by `stop`, so a speaking thread knows it was stopped.
    generation: u64,
    /// The answer a stop cut: the rest of it that still streams in is not
    /// read, until that conversation asks again (`started`).
    hushed: Option<String>,
}

impl Voice {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// More of `conversation`'s answer: speaks the sentences it finishes.
    pub(super) fn feed(&self, runner: &Arc<Runner>, conversation: &str, text: &str) {
        {
            let mut state = self.state();
            if state.hushed.as_deref() == Some(conversation) {
                return;
            }
            if state.conversation.as_deref() != Some(conversation) {
                *state = State {
                    conversation: Some(conversation.to_owned()),
                    generation: state.generation + 1,
                    hushed: state.hushed.take(),
                    ..State::default()
                };
            }
            state.buffer.push_str(text);
            while let Some(end) = sentence_end(&state.buffer) {
                let sentence: String = state.buffer.drain(..end).collect();
                let sentence = sentence.trim().to_owned();
                if !sentence.is_empty() {
                    state.queue.push_back(sentence);
                    state.started = true;
                }
            }
        }
        self.speak_queue(runner);
    }

    /// `conversation`'s answer is complete: speaks what is left, or all of
    /// `text` when nothing streamed.
    pub(super) fn finish(&self, runner: &Arc<Runner>, conversation: &str, text: &str) {
        {
            let mut state = self.state();
            if state.hushed.as_deref() == Some(conversation) {
                return;
            }
            if state.conversation.as_deref() != Some(conversation) || !state.started {
                *state = State {
                    conversation: Some(conversation.to_owned()),
                    generation: state.generation + 1,
                    buffer: text.to_owned(),
                    hushed: state.hushed.take(),
                    ..State::default()
                };
                while let Some(end) = sentence_end(&state.buffer) {
                    let sentence: String = state.buffer.drain(..end).collect();
                    state.queue.push_back(sentence.trim().to_owned());
                }
            }
            let rest = std::mem::take(&mut state.buffer);
            if !rest.trim().is_empty() {
                state.queue.push_back(rest.trim().to_owned());
            }
            state.queue.retain(|sentence| !sentence.is_empty());
            state.started = true;
            state.finished = true;
        }
        self.speak_queue(runner);
    }

    /// Whether an answer is still being read here: sentences wait their
    /// turn, the speaking thread is handing one over, or the answer started
    /// streaming and more of it is still to come. The speech engine's own
    /// queue only holds sentences already handed to it, so between two
    /// sentences of a streamed or slowly made answer it can be empty while
    /// the answer goes on; conversation mode must not take that gap for the
    /// end of the reading.
    pub(super) fn reading(&self) -> bool {
        let state = self.state();
        state.speaking || !state.queue.is_empty() || (state.started && !state.finished)
    }

    /// Forgets `conversation`'s answer (it failed or was stopped) without
    /// stopping speech of another.
    pub(super) fn drop_answer(&self, conversation: &str) {
        let mut state = self.state();
        if state.conversation.as_deref() == Some(conversation) {
            let generation = state.generation + 1;
            *state = State {
                generation,
                hushed: state.hushed.take(),
                ..State::default()
            };
        }
    }

    /// Stops reading aloud and forgets the queue (lines waiting their turn
    /// stay: a question still waits for its answer). The rest of the answer
    /// being read, still streaming in, stays unread.
    pub(super) fn stop(&self) {
        let mut state = self.state();
        let generation = state.generation + 1;
        let hushed = state
            .conversation
            .take()
            .filter(|conversation| !conversation.starts_with("announce:"))
            .or_else(|| state.hushed.take());
        *state = State {
            generation,
            hushed,
            ..State::default()
        };
    }

    /// `conversation` asks again: its new answer is read.
    pub(super) fn started(&self, conversation: &str) {
        let mut state = self.state();
        if state.hushed.as_deref() == Some(conversation) {
            state.hushed = None;
        }
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Reads `text` aloud for `conversation` in its turn (`Priority`): never
    /// over what plays, nor while the person holds the shortcut. A newer
    /// line of the same conversation and priority replaces one still
    /// waiting. Questions and summaries are only shown while
    /// [`set_spoken_announcements`] is off.
    pub(super) fn announce(
        &self,
        runner: &Arc<Runner>,
        priority: Priority,
        conversation: &str,
        title: &str,
        text: &str,
    ) {
        if text.trim().is_empty() || priority != Priority::Answer && !SPOKEN.load(Ordering::Acquire)
        {
            return;
        }
        {
            let mut pending = self.pending();
            pending
                .items
                .retain(|item| !(item.conversation == conversation && item.priority == priority));
            pending.items.push(Announcement {
                priority,
                conversation: conversation.to_owned(),
                title: title.to_owned(),
                text: text.trim().to_owned(),
            });
            if pending.pumping {
                return;
            }
            pending.pumping = true;
        }
        let runner = Arc::clone(runner);
        let spawned = std::thread::Builder::new()
            .name("assistant-announce".into())
            .spawn(move || runner.voice.pump(&runner));
        if spawned.is_err() {
            self.pending().pumping = false;
        }
    }

    /// Speaks what waits, one at a time, whenever nothing plays.
    fn pump(&self, runner: &Arc<Runner>) {
        loop {
            if self.idle(runner) {
                let next = {
                    let mut pending = self.pending();
                    let next = next_announcement(&mut pending.items);
                    if next.is_none() {
                        pending.pumping = false;
                        return;
                    }
                    next
                };
                if let Some(next) = next {
                    let id = ANNOUNCED.fetch_add(1, Ordering::Relaxed);
                    // Its own reading, apart from any answer's.
                    self.finish(runner, &format!("announce:{id}"), &next.text);
                }
            } else if self.pending().items.is_empty() {
                self.pending().pumping = false;
                return;
            }
            (runner.hooks.sleep)(PUMP_EVERY);
        }
    }

    /// Nothing is read or waits to be read (an answer between two streamed
    /// sentences is still being read), and the person isn't speaking.
    fn idle(&self, runner: &Runner) -> bool {
        !self.reading() && !(runner.hooks.speaking)() && !(runner.hooks.announcements_wait)()
    }

    /// Starts the thread that speaks the queue, unless it runs.
    fn speak_queue(&self, runner: &Arc<Runner>) {
        {
            let mut state = self.state();
            if state.speaking || state.queue.is_empty() {
                return;
            }
            state.speaking = true;
        }
        let runner = Arc::clone(runner);
        let spawned = std::thread::Builder::new()
            .name("assistant-voice".into())
            .spawn(move || runner.voice.speak_all(&runner));
        if spawned.is_err() {
            self.state().speaking = false;
        }
    }

    fn speak_all(&self, runner: &Runner) {
        loop {
            let (sentence, voice, generation) = {
                let mut state = self.state();
                let Some(sentence) = state.queue.pop_front() else {
                    state.speaking = false;
                    return;
                };
                if state.voice.is_none() {
                    let (_, settings) = (runner.hooks.settings)();
                    let chosen = settings
                        .voice
                        .clone()
                        .or_else(|| (runner.hooks.pick_voice)(&sentence));
                    state.voice = Some(chosen);
                }
                (sentence, state.voice.clone().flatten(), state.generation)
            };
            if self.state().generation != generation {
                continue;
            }
            // Never into a hold or a dictation: an answer that arrives
            // while the person talks to the microphone waits for them.
            let waited_since = (runner.hooks.now_ms)();
            while (runner.hooks.person_talks)()
                && self.state().generation == generation
                && (runner.hooks.now_ms)().saturating_sub(waited_since) < LISTENING_AT_MOST_MS
            {
                (runner.hooks.sleep)(PUMP_EVERY);
            }
            if self.state().generation != generation {
                continue;
            }
            // Queued behind what plays (`speak_next`), so sentences follow
            // each other with no gap and the next is handed over at once.
            (runner.hooks.speak)(&sentence, voice.as_deref());
        }
    }
}

/// The person holds the Action shortcut or dictates: the microphone would
/// hear Agent-Desk. (Conversation mode's window waits for the reading itself.)
/// The runner asks through its `person_talks` hook.
pub(super) fn person_talks() -> bool {
    use crate::assistant::interaction::{Listening, listening};
    matches!(listening(), Listening::Hold | Listening::Dictation)
}

/// Where the first finished sentence in `text` ends (after its punctuation
/// and the space that follows), if one is finished and long enough.
fn sentence_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let boundary = match c {
            '\n' => true,
            '.' | '!' | '?' | '…' | ':' | ';' => {
                chars.peek().is_some_and(|(_, next)| next.is_whitespace())
            }
            _ => false,
        };
        if boundary {
            let end = index + c.len_utf8();
            if text[..end].trim().chars().count() >= MIN_SENTENCE || c == '\n' {
                return Some(end);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_end_at_punctuation_followed_by_space() {
        assert_eq!(sentence_end("Es ist 18 Uhr in Tokio. Und"), Some(23));
        assert_eq!(sentence_end("Es ist 18 Uhr in Tokio."), None);
        assert_eq!(sentence_end("Ja. Es ist 3.5 Grad warm. "), Some(25));
        assert_eq!(sentence_end("Zeile eins\nzwei"), Some(11));
        assert_eq!(sentence_end("Kurz"), None);
    }

    fn line(priority: Priority, conversation: &str, title: &str, text: &str) -> Announcement {
        Announcement {
            priority,
            conversation: conversation.into(),
            title: title.into(),
            text: text.into(),
        }
    }

    #[test]
    fn the_most_urgent_line_is_read_first_then_the_oldest() {
        let mut items = vec![
            line(Priority::Summary, "a", "Einstein", "Einstein ist fertig."),
            line(Priority::Question, "b", "Wetter", "Welche Stadt?"),
            line(
                Priority::Summary,
                "c",
                "Laptops",
                "Die Laptops sind fertig.",
            ),
            line(Priority::Answer, "d", "", "Okay, später."),
            line(Priority::Question, "e", "Mails", "An wen?"),
        ];
        let order: Vec<String> = std::iter::from_fn(|| next_announcement(&mut items))
            .map(|item| item.conversation)
            .collect();
        assert_eq!(order, ["d", "b", "e", "a", "c"]);
        assert_eq!(next_announcement(&mut items), None);
    }

    #[test]
    fn many_finished_jobs_are_told_in_one_line() {
        let mut items = vec![
            line(
                Priority::Summary,
                "a",
                "Einstein-Recherche",
                "Die Recherche ist fertig.",
            ),
            line(
                Priority::Summary,
                "b",
                "Mac mini Preise",
                "Der günstigste ist bei Juuz.",
            ),
            line(
                Priority::Question,
                "q",
                "Mails",
                "An wen soll ich schreiben?",
            ),
            line(Priority::Summary, "c", "Wetter Berlin", "Es wird sonnig."),
        ];
        assert_eq!(next_announcement(&mut items).unwrap().conversation, "q");
        let told = next_announcement(&mut items).unwrap();
        assert_eq!(
            told.text,
            "3 Aufträge sind fertig: Einstein-Recherche, Mac mini Preise und Wetter Berlin."
        );
        assert!(items.is_empty());
        // Two are still read one by one.
        let mut two = vec![
            line(Priority::Summary, "a", "Laptops", "The laptops are done."),
            line(Priority::Summary, "b", "Weather", "It will be sunny."),
        ];
        assert_eq!(
            next_announcement(&mut two).unwrap().text,
            "The laptops are done."
        );
        let mut three = vec![
            line(
                Priority::Summary,
                "a",
                "Laptops",
                "The laptop research is done.",
            ),
            line(
                Priority::Summary,
                "b",
                "Weather",
                "It will be sunny in Berlin.",
            ),
            line(
                Priority::Summary,
                "c",
                "Mail",
                "The drafts are saved in Gmail.",
            ),
        ];
        assert_eq!(
            next_announcement(&mut three).unwrap().text,
            "3 jobs are done: Laptops, Weather and Mail."
        );
    }

    #[test]
    fn the_thinking_line_comes_only_after_two_silent_seconds() {
        let start = 10_000;
        let after = THINKING_LINE_AFTER.as_millis() as u64;
        assert!(!thinking_line_due(start, start + after - 1, false, false));
        assert!(thinking_line_due(start, start + after, false, false));
        assert!(
            !thinking_line_due(start, start + after, true, false),
            "its answer already streams"
        );
        assert!(
            !thinking_line_due(start, start + after, false, true),
            "never over another line"
        );
        assert_eq!(thinking_line("What's on my calendar today?"), "On it.");
        assert_eq!(
            thinking_line("Bitte nimm die blaue Farbe und nicht die rote"),
            "Mach ich."
        );
    }

    #[test]
    fn a_hold_keeps_lines_waiting_until_it_ends() {
        set_listening(true, 1_000);
        assert!(listening(1_000));
        assert!(listening(1_000 + LISTENING_AT_MOST_MS - 1));
        assert!(!listening(1_000 + LISTENING_AT_MOST_MS));
        set_listening(false, 2_000);
        assert!(!listening(2_000));
    }
}
