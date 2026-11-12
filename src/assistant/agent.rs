//! The agent loop: sends the conversation and the desktop tools to the
//! provider, performs each tool call through `computer`, and repeats until
//! the model answers, the step limit is hit, or the request is cancelled.
//!
//! OWNER: brain.
