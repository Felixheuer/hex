//! The questions tasks wait on, one at a time, and where what the person
//! says next belongs.
//!
//! - **Inbox.** Several Background tasks may wait on the person
//!   (`TaskState::NeedsInput`) at once. Exactly one question is *active*:
//!   the one of the task the person last spoke to, else the oldest. The rest
//!   wait their turn; when the active one is answered or its task stops, the
//!   next becomes active (and is read aloud, `voice`).
//! - **Routing.** What the person says after a hold while a question is
//!   active answers it, unless it plainly addresses another task by name
//!   ("bei der Einstein-Recherche: nur Deutsch") or plainly starts something
//!   new ("neue Aufgabe: …", "recherchiere …" with nothing of the question
//!   in it). Only when the cheap signals disagree is it [`Route::Unsure`],
//!   for a quick model to settle (`Hooks::classify_reply`); otherwise it
//!   answers the active question.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use super::voice::Priority;
use super::{Runner, State, context};
use crate::assistant::types::{
    ConversationId, Event, Lane, Origin, Request, TaskState, TaskStatus,
};

/// One question a task waits on.
pub type Asked = crate::assistant::types::WaitingQuestion;

/// How long a quick model may take to place an utterance the router
/// can't (`Hooks::classify_reply`); after that it answers the question.
pub(super) const CLASSIFY_FOR: Duration = Duration::from_millis(1500);

/// The inbox in `state`.
pub(super) fn of(state: &State) -> Inbox {
    let asked = state
        .tasks
        .iter()
        .filter(|task| task.status.state == TaskState::NeedsInput)
        .filter_map(|task| {
            Some(Asked {
                conversation: task.status.conversation.clone(),
                title: task.status.title.clone(),
                question: task.question.clone()?,
                asked_ms: task.asked_ms.unwrap_or_default(),
            })
        })
        .collect();
    order(asked, state.last_addressed.as_deref())
}

impl Runner {
    pub(crate) fn inbox(&self) -> Inbox {
        of(&self.state())
    }

    /// Tells subscribers when the active question or the count changed
    /// (`Event::QuestionsChanged`), and reads a newly active question
    /// aloud when its task was asked by voice.
    pub(super) fn questions_changed(self: &Arc<Self>) {
        let (inbox, speak, several) = {
            let mut state = self.state();
            let inbox = of(&state);
            let told = inbox
                .active
                .as_ref()
                .map(|active| (active.conversation.clone(), inbox.waiting.len()));
            if state.told_inbox == told {
                return;
            }
            let was = state.told_inbox.as_ref().map(|(id, _)| id.clone());
            state.told_inbox = told;
            let speak = inbox
                .active
                .clone()
                .filter(|active| was.as_ref() != Some(&active.conversation))
                .filter(|active| {
                    state
                        .tasks
                        .iter()
                        .any(|task| task.status.conversation == active.conversation && task.voice)
                });
            let several = state
                .tasks
                .iter()
                .filter(|task| task.lane() == Lane::Background)
                .count()
                > 1;
            (inbox, speak, several)
        };
        self.emit(&Event::QuestionsChanged {
            active: inbox.active.clone(),
            waiting: inbox.waiting.len(),
        });
        if let Some(active) = speak
            && (self.hooks.settings)().1.speak_answers
        {
            // With several tasks about, the question says whose it is.
            let line = if several {
                format!("{}: {}", active.title, active.question)
            } else {
                active.question.clone()
            };
            self.voice.announce(
                self,
                Priority::Question,
                &active.conversation,
                &active.title,
                &line,
            );
        }
    }

    /// Places what the person said after a hold while a question is active
    /// (`route_reply`): it answers it (`None`, and `submit` hands it over),
    /// goes to another task or a new conversation (`None`, `request`
    /// changed), or was a command for the inbox itself, done here
    /// (`Some`, the active conversation).
    pub(super) fn route_hold(self: &Arc<Self>, request: &mut Request) -> Option<ConversationId> {
        if request.origin != Origin::ActionShortcut {
            return None;
        }
        let conversation = request.conversation.clone()?;
        let (active, others, inbox) = {
            let state = self.state();
            let inbox = of(&state);
            let active = inbox.active.clone()?;
            if active.conversation != conversation {
                return None;
            }
            let others: Vec<TaskStatus> = state
                .tasks
                .iter()
                .filter(|task| task.status.conversation != conversation)
                .map(|task| task.status.clone())
                .collect();
            (active, others, inbox)
        };
        let mut route = route_reply(&request.text, &active, &others);
        if route == Route::Unsure {
            route =
                match (self.hooks.classify_reply)(&active.question, &active.title, &request.text) {
                    Some(false) => Route::New,
                    _ => Route::Answer,
                };
        }
        let german = speaks_german(&request.text, &active.question);
        match route {
            Route::Answer | Route::Unsure => None,
            Route::Task(id) => {
                self.state().last_addressed = Some(id.clone());
                request.conversation = Some(id);
                None
            }
            Route::New => {
                request.conversation = None;
                None
            }
            Route::List => {
                let line = list_line(&inbox, german);
                self.voice
                    .announce(self, Priority::Answer, &conversation, "", &line);
                Some(conversation)
            }
            Route::Defer => {
                let line = if german {
                    "Okay, später."
                } else {
                    "Okay, later."
                };
                self.voice
                    .announce(self, Priority::Answer, &conversation, "", line);
                self.defer(&conversation);
                Some(conversation)
            }
        }
    }

    /// Puts `conversation`'s question behind every other one, and brings
    /// up the next (`Event::QuestionsChanged`).
    pub(crate) fn defer(self: &Arc<Self>, conversation: &str) {
        let now = self.now();
        {
            let mut state = self.state();
            // Behind every other question, even within a millisecond.
            let last = state.tasks.iter().filter_map(|task| task.asked_ms).max();
            let now = last.map_or(now, |last| now.max(last + 1));
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.status.conversation == conversation)
            {
                task.asked_ms = Some(now);
            }
            if state.last_addressed.as_deref() == Some(conversation) {
                state.last_addressed = None;
            }
        }
        self.questions_changed();
    }
}

/// Whether to answer the person in German: what they said, else the
/// question, reads German.
fn speaks_german(said: &str, question: &str) -> bool {
    let of = crate::assistant::speech::language_of;
    of(said).or_else(|| of(question)) == Some("de")
}

/// The waiting questions as one spoken line.
pub(super) fn list_line(inbox: &Inbox, german: bool) -> String {
    let all: Vec<&Asked> = inbox.active.iter().chain(inbox.waiting.iter()).collect();
    if all.is_empty() {
        return if german {
            "Keine Fragen offen.".into()
        } else {
            "No questions are waiting.".into()
        };
    }
    let head = match (all.len(), german) {
        (1, true) => "Eine Frage ist offen.".to_owned(),
        (1, false) => "One question is waiting.".to_owned(),
        (n, true) => format!("{} Fragen sind offen.", number(n, true)),
        (n, false) => format!("{} questions are waiting.", number(n, false)),
    };
    let items: Vec<String> = all
        .iter()
        .map(|asked| format!("{}: {}", asked.title, asked.question))
        .collect();
    format!("{head} {}", items.join(" "))
}

/// A small count as a word, capitalised ("Drei", "Three").
pub(super) fn number(n: usize, german: bool) -> String {
    const DE: [&str; 11] = [
        "Null", "Eine", "Zwei", "Drei", "Vier", "Fünf", "Sechs", "Sieben", "Acht", "Neun", "Zehn",
    ];
    const EN: [&str; 11] = [
        "No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten",
    ];
    let words = if german { DE } else { EN };
    words
        .get(n)
        .map_or_else(|| n.to_string(), |word| (*word).to_owned())
}

/// The questions waiting on the person: the one to answer now, and the
/// others in the order they come up.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Inbox {
    pub active: Option<Asked>,
    pub waiting: Vec<Asked>,
}

impl Inbox {
    /// How many questions wait, the active one included.
    pub fn len(&self) -> usize {
        self.waiting.len() + usize::from(self.active.is_some())
    }
}

/// Orders `asked`: the question of `last_addressed` (the conversation the
/// person last spoke to) first, then oldest first.
pub(super) fn order(mut asked: Vec<Asked>, last_addressed: Option<&str>) -> Inbox {
    asked.sort_by_key(|question| {
        (
            Some(question.conversation.as_str()) != last_addressed,
            question.asked_ms,
        )
    });
    let mut asked = asked.into_iter();
    Inbox {
        active: asked.next(),
        waiting: asked.collect(),
    }
}

/// Where an utterance said while a question is active belongs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Route {
    /// It answers the active question.
    Answer,
    /// It is for this other task (a waiting question, or a running one to
    /// steer).
    Task(ConversationId),
    /// It is a new request.
    New,
    /// The cheap signals disagree: a quick model decides, else it answers.
    Unsure,
    /// "Welche Fragen sind offen?": read the waiting questions aloud.
    List,
    /// "Später" / "überspring die Frage": the active question goes to the
    /// back of the inbox, and the next one comes up.
    Defer,
}

/// Where `text` belongs while `active` waits, among the tasks `others`
/// (every task but the active one's).
pub fn route_reply(text: &str, active: &Asked, others: &[TaskStatus]) -> Route {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() {
        return Route::Answer;
    }
    if asks_for_list(&lower) {
        return Route::List;
    }
    if defers(&lower) {
        return Route::Defer;
    }
    if starts_new(&lower) {
        return Route::New;
    }
    if let Some(task) = context::route(text, others) {
        // Named the active task too ("bei der Recherche" when both are
        // research): the question it waits on wins.
        if !names(&lower, &active.title) {
            return Route::Task(task);
        }
    }
    let said = context::words(text);
    let asked: HashSet<String> = context::words(&active.question)
        .union(&context::words(&active.title))
        .cloned()
        .collect();
    let shares = said.intersection(&asked).next().is_some();
    let count = lower.split_whitespace().count();
    if is_reply_word(&lower) || shares || count <= SHORT {
        return Route::Answer;
    }
    if starts_with_command(&lower) {
        return Route::Unsure;
    }
    Route::Answer
}

/// An utterance this short is an answer ("Deutsch", "an max@firma.de",
/// "die zweite Option bitte").
const SHORT: usize = 6;

/// "Welche Fragen sind offen?", "what's waiting for me?"
fn asks_for_list(lower: &str) -> bool {
    const ASKS: &[&str] = &[
        "welche fragen",
        "was ist offen",
        "was wartet",
        "offene fragen",
        "wie viele fragen",
        "which questions",
        "what questions",
        "open questions",
        "what's waiting",
        "what is waiting",
        "how many questions",
    ];
    ASKS.iter().any(|ask| lower.contains(ask))
}

/// "Später", "überspring die Frage", "skip", "not now": the whole
/// utterance is about putting the question off.
fn defers(lower: &str) -> bool {
    const DEFER: &[&str] = &[
        "später",
        "spaeter",
        "nachher",
        "jetzt nicht",
        "nicht jetzt",
        "überspring",
        "ueberspring",
        "nächste frage",
        "naechste frage",
        "later",
        "not now",
        "skip",
        "next question",
    ];
    let words = lower.split_whitespace().count();
    words <= 5 && DEFER.iter().any(|word| lower.contains(word))
}

/// It opens with words that start something new.
fn starts_new(lower: &str) -> bool {
    const OPENINGS: &[&str] = &[
        "neue aufgabe",
        "neuer auftrag",
        "eine neue aufgabe",
        "neue frage",
        "etwas anderes",
        "was anderes",
        "andere sache",
        "new task",
        "new job",
        "a new task",
        "new question",
        "something else",
        "different thing",
    ];
    OPENINGS.iter().any(|opening| {
        lower
            .strip_prefix(opening)
            .is_some_and(|rest| rest.is_empty() || !rest.starts_with(char::is_alphanumeric))
    })
}

/// It names the task titled `title` (shares a meaningful word with it).
fn names(lower: &str, title: &str) -> bool {
    context::words(lower)
        .intersection(&context::words(title))
        .next()
        .is_some()
}

/// A word that is plainly a reply: yes, no, a language, a choice.
fn is_reply_word(lower: &str) -> bool {
    const REPLIES: &[&str] = &[
        "ja", "nein", "jo", "nee", "genau", "richtig", "okay", "ok", "klar", "gerne", "passt",
        "stimmt", "yes", "no", "yep", "nope", "sure", "right", "correct", "fine", "deutsch",
        "englisch", "german", "english", "beide", "both", "keine", "none", "erste", "zweite",
        "dritte", "first", "second", "third",
    ];
    let first = lower
        .split(|c: char| !c.is_alphanumeric())
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    REPLIES.contains(&first) || lower.contains('@')
}

/// It opens like a new instruction ("recherchiere …", "open …").
fn starts_with_command(lower: &str) -> bool {
    const VERBS: &[&str] = &[
        "recherchier",
        "such",
        "find",
        "öffne",
        "oeffne",
        "starte",
        "schreib",
        "erstell",
        "mach",
        "vergleich",
        "plane",
        "buche",
        "bestell",
        "research",
        "search",
        "look",
        "open",
        "start",
        "write",
        "create",
        "make",
        "compare",
        "plan",
        "book",
        "order",
        "send",
        "schick",
    ];
    let first = lower
        .split(|c: char| !c.is_alphanumeric())
        .find(|word| !word.is_empty())
        .unwrap_or_default();
    VERBS.iter().any(|verb| first.starts_with(verb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::types::{Origin, TaskState};

    fn asked(id: &str, title: &str, question: &str, at: u64) -> Asked {
        Asked {
            conversation: id.into(),
            title: title.into(),
            question: question.into(),
            asked_ms: at,
        }
    }

    fn task(id: &str, title: &str) -> TaskStatus {
        TaskStatus {
            conversation: id.into(),
            origin: Origin::Background,
            title: title.into(),
            state: TaskState::Running,
            step: None,
            windows: Vec::new(),
            colour: 0,
        }
    }

    #[test]
    fn the_oldest_question_is_active_unless_the_person_spoke_to_another() {
        let questions = vec![
            asked("b", "Mac mini Preise", "Welches Modell?", 20),
            asked("a", "Einstein-Recherche", "Deutsch oder Englisch?", 10),
            asked("c", "Wetter", "Welche Stadt?", 30),
        ];
        let inbox = order(questions.clone(), None);
        assert_eq!(inbox.active.as_ref().unwrap().conversation, "a");
        assert_eq!(
            inbox
                .waiting
                .iter()
                .map(|q| q.conversation.as_str())
                .collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert_eq!(inbox.len(), 3);
        let inbox = order(questions, Some("c"));
        assert_eq!(inbox.active.unwrap().conversation, "c");
        assert_eq!(order(Vec::new(), Some("a")), Inbox::default());
    }

    #[test]
    fn short_replies_and_choices_answer_the_active_question() {
        let active = asked("a", "Einstein-Recherche", "Auf Deutsch oder Englisch?", 1);
        let others = [task("b", "Mac mini Preise vergleichen")];
        for reply in [
            "Deutsch",
            "Ja, bitte",
            "Nein",
            "Englisch reicht",
            "an max@firma.de",
            "yes please",
            "die zweite Option bitte",
            "Schreib den Bericht lieber auf Englisch, dann kann ich ihn weiterleiten",
        ] {
            assert_eq!(
                route_reply(reply, &active, &others),
                Route::Answer,
                "{reply}"
            );
        }
    }

    #[test]
    fn naming_another_task_goes_to_that_task() {
        let active = asked("a", "Einstein-Recherche", "Auf Deutsch oder Englisch?", 1);
        let others = [
            task("b", "Mac mini Preise vergleichen"),
            task("c", "Wetter Berlin"),
        ];
        assert_eq!(
            route_reply("Bei der Mac mini Aufgabe: nur den M5 Pro", &active, &others),
            Route::Task("b".into())
        );
        assert_eq!(
            route_reply("For the weather task, use Berlin Mitte", &active, &others),
            Route::Task("c".into())
        );
        // Naming the active task's own topic stays with its question.
        assert_eq!(
            route_reply("Bei der Einstein Aufgabe: Deutsch", &active, &others),
            Route::Answer
        );
    }

    #[test]
    fn the_inbox_is_driven_by_voice() {
        let active = asked("a", "Einstein-Recherche", "Auf Deutsch oder Englisch?", 1);
        for list in [
            "Welche Fragen sind offen?",
            "Was wartet noch auf mich?",
            "What questions are open?",
        ] {
            assert_eq!(route_reply(list, &active, &[]), Route::List, "{list}");
        }
        for defer in [
            "Später",
            "Überspring die Frage",
            "nicht jetzt",
            "skip",
            "later please",
            "nächste Frage",
        ] {
            assert_eq!(route_reply(defer, &active, &[]), Route::Defer, "{defer}");
        }
        // "Später" inside a longer instruction doesn't put the question off.
        assert_ne!(
            route_reply(
                "Schick die Mails später am Nachmittag an das ganze Team raus",
                &active,
                &[]
            ),
            Route::Defer
        );
    }

    #[test]
    fn a_new_request_is_told_apart() {
        let active = asked("a", "Einstein-Recherche", "Auf Deutsch oder Englisch?", 1);
        assert_eq!(
            route_reply(
                "Neue Aufgabe: buche mir einen Tisch für heute Abend",
                &active,
                &[]
            ),
            Route::New
        );
        assert_eq!(
            route_reply("New task: draft a reply to Anna's email", &active, &[]),
            Route::New
        );
        // A long command with nothing of the question in it is unsure.
        assert_eq!(
            route_reply(
                "Recherchiere die drei besten Laptops für Videoschnitt unter 2000 Euro",
                &active,
                &[]
            ),
            Route::Unsure
        );
        assert_eq!(
            route_reply(
                "Open my calendar and find a free hour next week for a call",
                &active,
                &[]
            ),
            Route::Unsure
        );
        // "Neue" inside a word or later in the sentence doesn't count.
        assert_eq!(
            route_reply("Neue Quellen sind okay", &active, &[]),
            Route::Answer
        );
    }
}
