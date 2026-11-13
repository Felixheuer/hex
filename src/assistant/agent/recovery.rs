//! What a long job survives without the person: the network going away
//! (it waits until the backend's service answers again, then goes on in
//! the same session), and a backend that ended without an answer (the
//! computer slept, the screen locked, the process was killed): it goes on
//! from where the session and the journal say it was. A run that keeps
//! failing without progress stops as before. Usage limits are the
//! runner's (`service::run`), a restart's resume is `JobKind::Resume`.
//!
//! OWNER: brain.

use std::time::Duration;

/// Failures in a row, without a step between them, a job goes on after.
pub const MAX_RETRIES: u32 = 6;
/// The longest a job waits for the network at a time.
pub const MAX_OFFLINE: Duration = Duration::from_secs(6 * 60 * 60);
/// How often it looks whether the network is back.
pub const POLL: Duration = Duration::from_secs(30);

/// What a backend's failure means for a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The network is down or the service unreachable: wait, then go on.
    Network,
    /// The backend ended without an answer: go on.
    Ended,
    /// Anything else (a refusal, a bad setting): it fails as before.
    Other,
}

/// What `message` (a backend's error, `{error:#}`) means.
pub fn classify(message: &str) -> Failure {
    let lower = message.to_lowercase();
    const NETWORK: &[&str] = &[
        "connection error",
        "connection refused",
        "connection reset",
        "network",
        "econnreset",
        "econnrefused",
        "enotfound",
        "eai_again",
        "etimedout",
        "enetunreach",
        "ehostunreach",
        "getaddrinfo",
        "fetch failed",
        "socket hang up",
        "dns",
        "offline",
        "timed out",
        "could not resolve host",
    ];
    if NETWORK.iter().any(|word| lower.contains(word)) {
        return Failure::Network;
    }
    const ENDED: &[&str] = &[
        "stopped without an answer",
        "stopped with exit code",
        "stopped before it answered",
        "it stopped without an answer",
        "signal: 9",
        "killed",
        "terminated",
    ];
    if ENDED.iter().any(|word| lower.contains(word)) {
        return Failure::Ended;
    }
    Failure::Other
}

/// Whether the backend's service answers: a connection to the Claude API
/// within a few seconds (any backend's service is reached the same way).
pub fn online() -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(addresses) = ("api.anthropic.com", 443).to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, Duration::from_secs(4)).is_ok())
}

/// What a job is told when it goes on after `failure`.
pub fn note(failure: Failure) -> &'static str {
    match failure {
        Failure::Network => {
            "The network went away and your last turn ended; it is back now. Check what was already done (plan.md, journal.md, your files) and go on; never repeat a send or submit you may already have done. A browser page that didn't load: open it again."
        }
        _ => {
            "Your last turn ended unexpectedly (the computer may have slept or its screen locked). Check what was already done (plan.md, journal.md, your files) and go on; never repeat a send or submit you may already have done. An app or browser that stopped responding: reopen it and go back to where you were."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_network_and_an_ended_backend_are_told_apart() {
        assert_eq!(
            classify("Claude Code: API Error: Connection error. (ECONNRESET)"),
            Failure::Network
        );
        assert_eq!(
            classify("getaddrinfo ENOTFOUND api.anthropic.com"),
            Failure::Network
        );
        assert_eq!(
            classify("Claude Code stopped with exit code 143 and no answer"),
            Failure::Ended
        );
        assert_eq!(classify("Claude Code: Invalid model name"), Failure::Other);
        assert!(note(Failure::Network).contains("never repeat a send"));
    }
}
