//! Why a quick turn failed, read from the brain's message: a short line
//! the Here bubble shows and Hex says, in the language of the question
//! (`assistant::lines`), and the one thing to do about it. Plain data, so
//! every rule is a unit test.

/// What went wrong, as far as the message tells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    /// The assistant is switched off.
    Off,
    /// Its setup isn't finished.
    Setup,
    /// The agent isn't signed in, or its key is missing.
    SignIn,
    /// The agent, or a tool it needs, isn't installed.
    Install,
    /// macOS doesn't allow something it needs.
    Permission,
    /// The agent hit a usage or rate limit.
    Limit,
    /// Anything else: it stopped, crashed, or timed out.
    Crash,
}

/// The one action a failure's bubble offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Remedy {
    /// Fix: Assistant settings (where the switch is).
    Settings,
    /// Fix: the setup, at the page that repairs what's missing.
    Repair,
    /// Try again: the same question once more.
    Retry,
}

impl Failure {
    /// The failure `message` (the brain's words, in English) reports.
    pub(crate) fn of(message: &str) -> Self {
        let lower = message.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
        if has(&["turn on the assistant", "the assistant is off"]) {
            Self::Off
        } else if has(&["finish setting up"]) {
            Self::Setup
        } else if crate::assistant::provider::limit::detect(message).is_some() {
            Self::Limit
        } else if has(&[
            "isn't signed in",
            "not signed in",
            "not logged in",
            "/login",
            "sign in",
            "log in",
            "api key",
            "unauthorized",
            "authentication",
            "oauth token",
            "401",
        ]) {
            Self::SignIn
        } else if has(&[
            "isn't installed",
            "not installed",
            "can't find it",
            "command not found",
            "no such file",
        ]) {
            Self::Install
        } else if has(&[
            "permission",
            "not permitted",
            "not allowed",
            "accessibility",
            "screen recording",
            "automation",
        ]) {
            Self::Permission
        } else {
            Self::Crash
        }
    }

    /// Its line, in English (translated where it's shown or said).
    pub(crate) fn line(self) -> &'static str {
        match self {
            Self::Off => "The assistant is off",
            Self::Setup => "Finish setting up the assistant first",
            Self::SignIn => "Your agent isn't signed in.",
            Self::Install => "Something the assistant needs isn't installed.",
            Self::Permission => "The assistant is missing a permission.",
            Self::Limit => "Your agent hit its usage limit.",
            Self::Crash => "That didn't work.",
        }
    }

    pub(crate) fn remedy(self) -> Remedy {
        match self {
            Self::Off => Remedy::Settings,
            Self::Setup | Self::SignIn | Self::Install | Self::Permission => Remedy::Repair,
            Self::Limit | Self::Crash => Remedy::Retry,
        }
    }

    /// Whether the brain's own message says more than the line (which agent,
    /// which tool, what crashed), so it shows small beneath it.
    pub(crate) fn shows_detail(self) -> bool {
        !matches!(self, Self::Off | Self::Setup)
    }
}

impl Remedy {
    /// The button's label, in English.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Settings | Self::Repair => "Fix",
            Self::Retry => "Try again",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brains_messages_have_their_failure_and_remedy() {
        let cases = [
            (
                "Turn on the assistant first.",
                Failure::Off,
                Remedy::Settings,
            ),
            (
                "Finish setting up the assistant first (Assistant settings).",
                Failure::Setup,
                Remedy::Repair,
            ),
            (
                "Claude Code isn't signed in.",
                Failure::SignIn,
                Remedy::Repair,
            ),
            (
                "No Anthropic API key is saved. Add one in Assistant settings, or choose another agent.",
                Failure::SignIn,
                Remedy::Repair,
            ),
            (
                "Codex isn't installed on this Mac, or Hex can't find it. Install it, or choose another agent in Assistant settings.",
                Failure::Install,
                Remedy::Repair,
            ),
            (
                "agent-desktop isn't installed, so the assistant can't use apps.",
                Failure::Install,
                Remedy::Repair,
            ),
            (
                "Hex may not control Mail: turn it on in System Settings › Privacy & Security › Automation",
                Failure::Permission,
                Remedy::Repair,
            ),
            (
                "Claude Code hit its usage limit: Claude AI usage limit reached|1760000000",
                Failure::Limit,
                Remedy::Retry,
            ),
            (
                "The task stopped unexpectedly.",
                Failure::Crash,
                Remedy::Retry,
            ),
        ];
        for (message, failure, remedy) in cases {
            assert_eq!(Failure::of(message), failure, "{message}");
            assert_eq!(failure.remedy(), remedy, "{message}");
        }
    }

    #[test]
    fn every_line_and_label_has_german() {
        let failures = [
            Failure::Off,
            Failure::Setup,
            Failure::SignIn,
            Failure::Install,
            Failure::Permission,
            Failure::Limit,
            Failure::Crash,
        ];
        let english = failures
            .iter()
            .map(|failure| failure.line())
            .chain([Remedy::Settings, Remedy::Retry].map(Remedy::label));
        for line in english {
            assert_ne!(
                crate::desktop::i18n::tr_in(Some("de"), line),
                line,
                "{line} has no German"
            );
        }
    }
}
