//! The two lines an agent starts its reply with to tell Agent-Desk something
//! instead of answering:
//!
//! - `HEX_JOB: <title>` then a short spoken line: the request is a job, so
//!   Agent-Desk continues it as a Background job (planning first). More
//!   `HEX_JOB: <title> | <the job>` lines right after it start further jobs
//!   beside it, each in a conversation of its own.
//! - `HEX_QUESTION:` then one batched question: the job needs the person's
//!   answer before it goes on.
//! - `HEX_CONFIRM: <category> <target>` then the question: Agent-Desk refused a
//!   consequential action (`NEEDS_CONFIRMATION`) and the person's yes allows
//!   that kind of action for the task (`autonomy`).
//!
//! - `HEX_UNDERSTOOD: <line>` before everything else in a quick turn's
//!   reply: what the agent understood the request as, which the island
//!   shows as its first reasoning line (`Event::Understood`) before the
//!   agent acts. It is never part of the answer ([`strip_understood`]).
//!
//! They work the same on every backend (no tool needed), and `Gate` keeps
//! them out of the streamed answer.

const JOB: &str = "HEX_JOB:";
const QUESTION: &str = "HEX_QUESTION:";
const CONFIRM: &str = "HEX_CONFIRM:";
const UNDERSTOOD: &str = "HEX_UNDERSTOOD:";

/// The longest understood line shown, in characters.
pub const UNDERSTOOD_CHARS: usize = 80;

/// The person's own quick turn's rule for the understood line (system
/// prompt).
pub const UNDERSTOOD_RULE: &str = "First line of every reply, before any tool call: `HEX_UNDERSTOOD: <the request restated with its resolved details (who, what, when), at most 80 characters, in the person's language>`, e.g. `HEX_UNDERSTOOD: Nopa, 2 people, tonight around 8 pm`. It is shown while you work and removed from your answer.\n";

/// `text` without its understood lines: the answer itself.
pub fn strip_understood(text: &str) -> String {
    if !text.contains(UNDERSTOOD) {
        return text.to_owned();
    }
    text.lines()
        .filter(|line| !line.trim_start().starts_with(UNDERSTOOD))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_start()
        .to_owned()
}

/// An understood line's words, trimmed and cut to [`UNDERSTOOD_CHARS`].
fn understood_words(line: &str) -> Option<String> {
    let line = line.trim().trim_matches(['“', '”', '"']).trim();
    (!line.is_empty()).then(|| crate::assistant::store::shorten(line, UNDERSTOOD_CHARS))
}

/// What a reply asks of Agent-Desk.
#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
    Job {
        title: String,
        reply: String,
        /// Further jobs to start beside it: (title, what each does).
        more: Vec<(String, String)>,
    },
    Question(String),
    Confirm {
        category: crate::assistant::autonomy::Category,
        question: String,
    },
}

/// The marker `text` starts with, if any.
pub fn parse(text: &str) -> Option<Marker> {
    let text = text.trim_start();
    if let Some(rest) = text.strip_prefix(JOB) {
        let (first, mut reply) = rest.split_once('\n').unwrap_or((rest, ""));
        let (title, _) = first.split_once('|').unwrap_or((first, ""));
        let mut more = Vec::new();
        while let Some(line) = reply.trim_start().strip_prefix(JOB) {
            let (line, after) = line.split_once('\n').unwrap_or((line, ""));
            if let Some((title, task)) = line.split_once('|')
                && !task.trim().is_empty()
            {
                more.push((job_title(title), task.trim().to_owned()));
            }
            reply = after;
        }
        return Some(Marker::Job {
            title: job_title(title),
            reply: reply.trim().to_owned(),
            more,
        });
    }
    if let Some(rest) = text.strip_prefix(CONFIRM) {
        let (head, question) = rest.split_once('\n').unwrap_or((rest, ""));
        let category = head
            .split_whitespace()
            .next()
            .and_then(crate::assistant::autonomy::Category::parse);
        let question = question.trim();
        return match category {
            Some(category) if !question.is_empty() => Some(Marker::Confirm {
                category,
                question: question.to_owned(),
            }),
            // Without a category it is still a question to the person.
            _ => (!question.is_empty()).then(|| Marker::Question(question.to_owned())),
        };
    }
    let rest = text.strip_prefix(QUESTION)?;
    let question = rest.trim();
    (!question.is_empty()).then(|| Marker::Question(question.to_owned()))
}

fn job_title(title: &str) -> String {
    let title = title.trim().trim_matches(['“', '”', '"']).trim();
    if title.is_empty() {
        "Job".into()
    } else {
        crate::assistant::store::shorten(title, 60)
    }
}

/// Holds back the start of each piece of streamed text until it can't be a
/// marker, and swallows a piece that is one.
#[derive(Debug, Default)]
pub struct Gate {
    /// Text of the current piece not yet let through.
    held: String,
    /// The current piece is a marker: swallow it.
    marker: bool,
    /// The current piece isn't a marker: let it through.
    open: bool,
    /// An understood line read from the stream, not yet taken.
    understood: Option<String>,
}

impl Gate {
    /// The text to show for `delta`, if any. A delta that is only a
    /// paragraph break starts a new piece (text after a tool call).
    pub fn pass(&mut self, delta: &str) -> Option<String> {
        if delta == "\n\n" {
            // An understood line the piece ended on, without its newline.
            if let Some(line) = self.held.trim_start().strip_prefix(UNDERSTOOD)
                && let Some(words) = understood_words(line)
            {
                self.understood = Some(words);
            }
            self.held.clear();
            self.marker = false;
            self.open = false;
            return Some(delta.to_owned());
        }
        if self.marker {
            return None;
        }
        if self.open {
            return Some(delta.to_owned());
        }
        self.held.push_str(delta);
        loop {
            let start = self.held.trim_start();
            // The understood line: read once it is whole, then the rest of
            // the piece goes on as if it had started there.
            if let Some(rest) = start.strip_prefix(UNDERSTOOD) {
                let (line, after) = rest.split_once('\n')?;
                if let Some(words) = understood_words(line) {
                    self.understood = Some(words);
                }
                self.held = after.to_owned();
                continue;
            }
            if start.starts_with(JOB) || start.starts_with(QUESTION) || start.starts_with(CONFIRM) {
                self.marker = true;
                self.held.clear();
                return None;
            }
            let could_be = JOB.starts_with(start)
                || QUESTION.starts_with(start)
                || CONFIRM.starts_with(start)
                || UNDERSTOOD.starts_with(start);
            if could_be {
                return None;
            }
            self.open = true;
            return Some(std::mem::take(&mut self.held));
        }
    }

    /// The understood line the stream gave since last asked, once.
    pub fn take_understood(&mut self) -> Option<String> {
        self.understood.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_and_a_question_are_read_from_the_reply() {
        assert_eq!(
            parse("HEX_JOB: Supplier research\nOn it: 50 suppliers, Gmail drafts, and a sheet."),
            Some(Marker::Job {
                title: "Supplier research".into(),
                reply: "On it: 50 suppliers, Gmail drafts, and a sheet.".into(),
                more: Vec::new(),
            })
        );
        assert_eq!(
            parse(
                "HEX_JOB: Mac mini prices | compare\nHEX_JOB: Einstein | Research Albert Einstein's life\nHEX_JOB: Weather | Find the weather in Berlin this weekend\nMach ich: drei Aufgaben laufen."
            ),
            Some(Marker::Job {
                title: "Mac mini prices".into(),
                reply: "Mach ich: drei Aufgaben laufen.".into(),
                more: vec![
                    ("Einstein".into(), "Research Albert Einstein's life".into()),
                    (
                        "Weather".into(),
                        "Find the weather in Berlin this weekend".into()
                    ),
                ],
            })
        );
        assert_eq!(
            parse("  HEX_QUESTION:\nWho is your colleague, and in which language?"),
            Some(Marker::Question(
                "Who is your colleague, and in which language?".into()
            ))
        );
        assert_eq!(
            parse("HEX_CONFIRM: send 3 emails to suppliers\nShall I send the three emails now?"),
            Some(Marker::Confirm {
                category: crate::assistant::autonomy::Category::Send,
                question: "Shall I send the three emails now?".into()
            })
        );
        assert_eq!(parse("It's 18:00 in Tokyo."), None);
        assert_eq!(parse("HEX_QUESTION:  "), None);
    }

    #[test]
    fn the_understood_line_is_read_from_the_stream_and_kept_out_of_the_answer() {
        let mut gate = Gate::default();
        let mut shown = String::new();
        let mut understood = Vec::new();
        for delta in [
            "HEX_UNDER",
            "STOOD: Nopa, 2 people,",
            " tonight 8 pm\nBooked",
            " it.",
        ] {
            shown.extend(gate.pass(delta));
            understood.extend(gate.take_understood());
        }
        assert_eq!(understood, ["Nopa, 2 people, tonight 8 pm"]);
        assert_eq!(shown, "Booked it.");
        // A marker on the next line is still one.
        let mut gate = Gate::default();
        let shown: Vec<_> = ["HEX_UNDERSTOOD: research\nHEX_JOB: Research\nOn it."]
            .iter()
            .filter_map(|delta| gate.pass(delta))
            .collect();
        assert!(shown.is_empty(), "{shown:?}");
        assert_eq!(gate.take_understood().as_deref(), Some("research"));
        // A line alone before a tool call: read at the piece's end.
        let mut gate = Gate::default();
        assert_eq!(gate.pass("HEX_UNDERSTOOD: the weather in Berlin"), None);
        assert_eq!(gate.pass("\n\n").as_deref(), Some("\n\n"));
        assert_eq!(
            gate.take_understood().as_deref(),
            Some("the weather in Berlin")
        );
        assert_eq!(gate.take_understood(), None, "once");
        // Long lines are cut.
        let mut gate = Gate::default();
        gate.pass(&format!("HEX_UNDERSTOOD: {}\n", "word ".repeat(40)));
        assert!(gate.take_understood().unwrap().chars().count() <= UNDERSTOOD_CHARS);
        // The final text loses it, so its markers still parse.
        assert_eq!(
            strip_understood("HEX_UNDERSTOOD: research\nHEX_JOB: Research\nOn it."),
            "HEX_JOB: Research\nOn it."
        );
        assert_eq!(
            parse(&strip_understood("HEX_UNDERSTOOD: who\nHEX_QUESTION: Who?")),
            Some(Marker::Question("Who?".into()))
        );
        assert_eq!(strip_understood("Plain answer."), "Plain answer.");
    }

    #[test]
    fn the_gate_keeps_markers_out_of_the_stream() {
        let mut gate = Gate::default();
        let shown: Vec<_> = ["HEX", "_JO", "B: Research\nOn it", "."]
            .iter()
            .filter_map(|delta| gate.pass(delta))
            .collect();
        assert!(shown.is_empty(), "{shown:?}");
        let mut gate = Gate::default();
        let shown: String = ["HE", "LLO there", ". ", "\n\n", "HEX_QUESTION: who?"]
            .iter()
            .filter_map(|delta| gate.pass(delta))
            .collect();
        assert_eq!(shown, "HELLO there. \n\n");
    }
}
