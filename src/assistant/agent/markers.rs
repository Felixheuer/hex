//! The two lines an agent starts its reply with to tell Hex something
//! instead of answering:
//!
//! - `HEX_JOB: <title>` then a short spoken line: the request is a job, so
//!   Hex continues it as a Background job (planning first).
//! - `HEX_QUESTION:` then one batched question: the job needs the person's
//!   answer before it goes on.
//! - `HEX_CONFIRM: <category> <target>` then the question: Hex refused a
//!   consequential action (`NEEDS_CONFIRMATION`) and the person's yes allows
//!   that kind of action for the task (`autonomy`).
//!
//! They work the same on every backend (no tool needed), and `Gate` keeps
//! them out of the streamed answer.

const JOB: &str = "HEX_JOB:";
const QUESTION: &str = "HEX_QUESTION:";
const CONFIRM: &str = "HEX_CONFIRM:";

/// What a reply asks of Hex.
#[derive(Clone, Debug, PartialEq)]
pub enum Marker {
    Job {
        title: String,
        reply: String,
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
        let (title, reply) = rest.split_once('\n').unwrap_or((rest, ""));
        let title = title.trim().trim_matches(['“', '”', '"']).trim();
        return Some(Marker::Job {
            title: if title.is_empty() {
                "Job".into()
            } else {
                crate::assistant::store::shorten(title, 60)
            },
            reply: reply.trim().to_owned(),
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
}

impl Gate {
    /// The text to show for `delta`, if any. A delta that is only a
    /// paragraph break starts a new piece (text after a tool call).
    pub fn pass(&mut self, delta: &str) -> Option<String> {
        if delta == "\n\n" {
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
        let start = self.held.trim_start();
        if start.starts_with(JOB) || start.starts_with(QUESTION) || start.starts_with(CONFIRM) {
            self.marker = true;
            self.held.clear();
            return None;
        }
        let could_be =
            JOB.starts_with(start) || QUESTION.starts_with(start) || CONFIRM.starts_with(start);
        if could_be {
            return None;
        }
        self.open = true;
        Some(std::mem::take(&mut self.held))
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
                reply: "On it: 50 suppliers, Gmail drafts, and a sheet.".into()
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
