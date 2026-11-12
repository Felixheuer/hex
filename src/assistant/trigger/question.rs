//! What one press of the Action shortcut asks: its lane, settled as it
//! goes down; the desktop read once the hold is real; and the request its
//! transcript makes.

use std::time::Duration;

use crate::assistant::computer::ContextHandle;
use crate::assistant::{Context, ConversationId, Lane, Origin, Request};
use crate::control::{PrintReply, PrintedTranscript};

/// How long a question waits for its transcript; a job always answers or
/// drops its reply long before this.
const TRANSCRIPT_WAIT: Duration = Duration::from_secs(300);

/// How long a quick question waits for the desktop read its hold started,
/// which has usually finished while the person spoke. Past it, it asks with
/// what has arrived (the Accessibility part) and drops the rest.
const CONTEXT_WAIT: Duration = Duration::from_secs(3);

/// Spoken openings that send a question to the Background lane, as words
/// compared without case or the punctuation around them. `true`: only with a
/// colon after it ("Hintergrund: …"), since "Hintergrund ändern" asks about
/// the background itself.
const SPOKEN_BACKGROUND: [(&[&str], bool); 5] = [
    (&["im", "hintergrund"], false),
    (&["in", "the", "background"], false),
    (&["nebenbei"], false),
    (&["parallel"], false),
    (&["hintergrund"], true),
];

/// Starts a desktop read on its own thread (`computer::context_async`).
type ReadDesktop = Box<dyn FnOnce() -> ContextHandle + Send>;

/// `text` without its spoken opening when it asks for the Background lane
/// ("Im Hintergrund, räum den Downloads-Ordner auf" becomes "räum den
/// Downloads-Ordner auf"); `None` when it doesn't start with one. A word
/// that merely starts like one ("Parallelogramm", "Hintergrundbild") is not
/// one.
pub(crate) fn spoken_background(text: &str) -> Option<&str> {
    let text = text.trim();
    SPOKEN_BACKGROUND.iter().find_map(|&(words, colon)| {
        let mut rest = text;
        let mut last = "";
        for &word in words {
            let trimmed = rest.trim_start();
            let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
            let (spoken, after) = trimmed.split_at(end);
            let bare = spoken.trim_matches(|c: char| !c.is_alphanumeric());
            if bare.to_lowercase() != word {
                return None;
            }
            last = spoken;
            rest = after;
        }
        // "Hintergrund:" or "Hintergrund :".
        if colon && !last.ends_with(':') {
            rest = rest.trim_start().strip_prefix(':')?;
        }
        Some(
            rest.trim_start_matches(|c: char| !c.is_alphanumeric())
                .trim_end(),
        )
    })
}

/// What one press of the Action shortcut asks, settled as it goes down.
pub(crate) struct Question {
    /// The press followed a tap: a new Background task.
    background: bool,
    /// How to read the desktop, until the hold is real (Here questions).
    read: Option<ReadDesktop>,
    /// The read under way; dropping it cancels it and frees its screenshot.
    context: Option<ContextHandle>,
    /// The hold became real.
    real: bool,
}

impl Question {
    /// Settles a press: a new Background task after a tap, else a Here
    /// question that reads the desktop with `read_desktop` once
    /// [`Question::intend`] says the hold is real. Nothing is read at the
    /// press itself.
    pub(crate) fn press(
        background: bool,
        read_desktop: impl FnOnce() -> ContextHandle + Send + 'static,
    ) -> Self {
        let read = (!background).then(|| Box::new(read_desktop) as ReadDesktop);
        Self {
            background,
            read,
            context: None,
            real: false,
        }
    }

    /// The lane the press asks in, as settled at the press.
    pub(crate) fn lane(&self) -> Lane {
        if self.background {
            Lane::Background
        } else {
            Lane::Here
        }
    }

    /// The hold passed the hold threshold without another key: start
    /// reading the desktop (once; never for a Background task). Returns
    /// whether the hold only now became real.
    pub(crate) fn intend(&mut self) -> bool {
        if let Some(read) = self.read.take() {
            self.context = Some(read());
        }
        !std::mem::replace(&mut self.real, true)
    }

    /// The request `text` makes, or none when nothing was said. A hold
    /// continues the conversation `focus` names (a task waiting on a
    /// question, a running Here task, or a recent one), and the assistant
    /// decides how to answer; a forced Background task, or words that ask
    /// for one first, start their own and look for themselves.
    pub(crate) fn request(
        mut self,
        text: &str,
        focus: impl FnOnce() -> Option<ConversationId>,
    ) -> Option<Request> {
        let text = text.trim();
        let background = |text: &str| {
            (!text.is_empty()).then(|| Request {
                conversation: None,
                text: text.into(),
                origin: Origin::Background,
                context: Context::default(),
            })
        };
        if text.is_empty() {
            return None;
        }
        if self.background {
            return background(text);
        }
        if let Some(rest) = spoken_background(text) {
            // Dropping the read cancels it: a Background task sends no
            // screenshot.
            return background(rest);
        }
        // A hold the loop never saw become real reads now.
        self.intend();
        let context = self
            .context
            .map(|context| context.wait(CONTEXT_WAIT))
            .unwrap_or_default();
        Some(Request {
            conversation: focus(),
            text: text.into(),
            origin: Origin::ActionShortcut,
            context,
        })
    }

    /// The reply the question's transcription answers. Once it does, the
    /// question's own thread builds the request and hands it to `submit`;
    /// a dropped reply (a discard, a cancel, a failure) or no words asks
    /// nothing.
    pub(crate) fn ask(
        self,
        focus: impl FnOnce() -> Option<ConversationId> + Send + 'static,
        submit: impl FnOnce(Request) + Send + 'static,
    ) -> PrintReply {
        let (reply, transcript) = PrintReply::channel();
        let spawned = std::thread::Builder::new()
            .name("hex-assistant-question".into())
            .spawn(move || {
                let Ok(PrintedTranscript::Text(text)) = transcript.recv_timeout(TRANSCRIPT_WAIT)
                else {
                    return;
                };
                if let Some(request) = self.request(&text, focus) {
                    submit(request);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not wait for a question's transcript");
        }
        reply
    }
}
