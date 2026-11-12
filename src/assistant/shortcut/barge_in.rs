//! What a barge-in's capture decides. A voice over an answer only pauses
//! the reading (`speech::pause`): on the built-in speakers the voice may
//! have been Hex's own. Once the capture is transcribed:
//!
//! - words of the person's own: the answer is dropped and they ask
//!   ("took over");
//! - a spoken stop: Hex stays quiet, as Escape does;
//! - nothing, or Hex's own words heard back: the reading resumes from the
//!   start of the sentence it cut.

/// What follows a barge-in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AfterBargeIn {
    /// Read on; why (for the log).
    Resume(&'static str),
    /// The person's words: drop the answer and ask.
    TakeOver,
    /// "Stopp": quiet for good.
    Stop,
}

/// What `transcript` (`None` when the capture gave nothing) means after a
/// barge-in; `heard_back` tells Hex's own reading.
pub(crate) fn after_barge_in(
    transcript: Option<&str>,
    heard_back: impl Fn(&str) -> bool,
) -> AfterBargeIn {
    let Some(text) = transcript.map(str::trim) else {
        return AfterBargeIn::Resume("empty");
    };
    if !text.chars().any(char::is_alphanumeric) {
        return AfterBargeIn::Resume("empty");
    }
    if super::spoken_stop::spoken_stop(text).is_some() {
        return AfterBargeIn::Stop;
    }
    if heard_back(text) {
        return AfterBargeIn::Resume("own echo");
    }
    AfterBargeIn::TakeOver
}

/// Carries out `after` for a reading paused by a barge-in, and logs it.
pub(crate) fn settle(after: AfterBargeIn) {
    match after {
        AfterBargeIn::Resume(why) => {
            tracing::info!(why, "barge-in: resumed");
            crate::assistant::speech::resume();
        }
        AfterBargeIn::TakeOver => {
            tracing::info!("barge-in: took over");
            crate::assistant::interaction::hush();
        }
        // The spoken stop runs its own path (Escape's).
        AfterBargeIn::Stop => tracing::info!("barge-in: stopped"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: &str) -> bool {
        false
    }

    #[test]
    fn nothing_heard_resumes_the_reading() {
        assert_eq!(after_barge_in(None, never), AfterBargeIn::Resume("empty"));
        assert_eq!(
            after_barge_in(Some("  "), never),
            AfterBargeIn::Resume("empty")
        );
        assert_eq!(
            after_barge_in(Some("…"), never),
            AfterBargeIn::Resume("empty")
        );
    }

    #[test]
    fn hex_s_own_voice_heard_back_resumes_the_reading() {
        assert_eq!(
            after_barge_in(
                Some("Hamburg ist mit rund eins komma neun Millionen."),
                |_| true
            ),
            AfterBargeIn::Resume("own echo")
        );
    }

    #[test]
    fn the_person_s_words_take_over_and_a_stop_stops() {
        assert_eq!(
            after_barge_in(Some("Und in Hamburg?"), never),
            AfterBargeIn::TakeOver
        );
        assert_eq!(after_barge_in(Some("Stopp!"), never), AfterBargeIn::Stop);
    }
}
