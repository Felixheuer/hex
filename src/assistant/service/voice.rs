//! Reading a Here answer aloud as it streams: each finished sentence is
//! queued and spoken in turn while the rest is still being written, so the
//! first words come out long before the whole answer is there.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::Runner;

/// A sentence shorter than this waits for the next one ("Ja." alone sounds
/// clipped).
const MIN_SENTENCE: usize = 12;

#[derive(Default)]
pub(super) struct Voice {
    state: Mutex<State>,
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
    /// The voice for this answer, chosen from its first sentence.
    voice: Option<Option<String>>,
    /// The thread speaking the queue is running.
    speaking: bool,
    /// Bumped by `stop`, so a speaking thread knows it was stopped.
    generation: u64,
}

impl Voice {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// More of `conversation`'s answer: speaks the sentences it finishes.
    pub(super) fn feed(&self, runner: &Arc<Runner>, conversation: &str, text: &str) {
        {
            let mut state = self.state();
            if state.conversation.as_deref() != Some(conversation) {
                *state = State {
                    conversation: Some(conversation.to_owned()),
                    generation: state.generation + 1,
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
            if state.conversation.as_deref() != Some(conversation) || !state.started {
                *state = State {
                    conversation: Some(conversation.to_owned()),
                    generation: state.generation + 1,
                    buffer: text.to_owned(),
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
        }
        self.speak_queue(runner);
    }

    /// Forgets `conversation`'s answer (it failed or was stopped) without
    /// stopping speech of another.
    pub(super) fn drop_answer(&self, conversation: &str) {
        let mut state = self.state();
        if state.conversation.as_deref() == Some(conversation) {
            let generation = state.generation + 1;
            *state = State {
                generation,
                ..State::default()
            };
        }
    }

    /// Stops reading aloud and forgets the queue.
    pub(super) fn stop(&self) {
        let mut state = self.state();
        let generation = state.generation + 1;
        *state = State {
            generation,
            ..State::default()
        };
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
            // Queued behind what plays (`speak_next`), so sentences follow
            // each other with no gap and the next is handed over at once.
            (runner.hooks.speak)(&sentence, voice.as_deref());
        }
    }
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
}
