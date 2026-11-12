//! Conversations on disk: one JSON file per conversation, and its screenshots
//! beside it, under `<support dir>/assistant/conversations/`.
//!
//! `<id>.json` holds the conversation, the backend sessions that continue
//! it, and how its last request ended (`Outcome`; `Running` with the pid of
//! the Hex that runs it); `<id>/` holds its PNG attachments. Both are private to this user, and
//! every write goes to a temporary file first, then replaces the old one.
//!
//! OWNER: brain.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use color_eyre::{Result, eyre::WrapErr};
use serde::{Deserialize, Serialize};

use super::types::{Conversation, ConversationSummary, Message, Origin, Outcome, Part, Role};

/// How long a title made from the first question may be, in characters.
const TITLE_CHARS: usize = 60;
/// How long a summary's preview may be, in characters.
const PREVIEW_CHARS: usize = 120;

/// A conversation as saved: the conversation itself, plus each backend's
/// session id (`claude_code`, `codex`, `open_code`) so the next question
/// continues the same session.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Stored {
    #[serde(flatten)]
    pub conversation: Conversation,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, String>,
    /// How its last request ended; `Running` while one runs.
    #[serde(default)]
    pub outcome: Outcome,
    /// The process running it, while `Running`: a dead one means Hex quit
    /// or crashed mid-task (`Interrupted`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// A job's folder (`agent::workspace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

impl Stored {
    /// A new, empty conversation titled after `question`.
    pub fn new(id: &str, origin: Origin, question: &str) -> Self {
        let now = now_ms();
        Self {
            conversation: Conversation {
                id: id.to_owned(),
                title: title(question),
                created_ms: now,
                updated_ms: now,
                origin,
                messages: Vec::new(),
            },
            sessions: BTreeMap::new(),
            outcome: Outcome::Answered,
            pid: None,
            workspace: None,
        }
    }

    /// Marks it running in this process.
    pub fn set_running(&mut self) {
        self.outcome = Outcome::Running;
        self.pid = Some(std::process::id());
    }

    /// Marks how it ended.
    pub fn set_outcome(&mut self, outcome: Outcome) {
        self.outcome = outcome;
        self.pid = None;
    }

    /// Its outcome as it stands now: `Running` whose process is gone reads
    /// as `Interrupted`.
    pub fn current_outcome(&self) -> Outcome {
        match (self.outcome, self.pid) {
            (Outcome::Running, Some(pid)) if !alive(pid) => Outcome::Interrupted,
            (Outcome::Running, None) => Outcome::Interrupted,
            (outcome, _) => outcome,
        }
    }

    /// The last question the person asked, for a continuation.
    pub fn last_question(&self) -> Option<String> {
        self.conversation
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(Message::text)
            .filter(|text| !text.trim().is_empty())
    }

    /// Adds `message` and marks the conversation updated.
    pub fn push(&mut self, message: Message) {
        self.conversation.updated_ms = self.conversation.updated_ms.max(message.at_ms);
        self.conversation.messages.push(message);
    }
}

/// A folder of conversations.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The conversations under Application Support.
    pub fn open() -> Result<Self> {
        Ok(Self::at(assistant_dir()?.join("conversations")))
    }

    /// Conversations in `dir` (tests).
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self, id: &str) -> Option<PathBuf> {
        valid_id(id).then(|| self.dir.join(format!("{id}.json")))
    }

    /// The saved conversation `id`, if there is one that reads.
    pub fn load(&self, id: &str) -> Option<Stored> {
        let data = fs::read(self.file(id)?).ok()?;
        match serde_json::from_slice(&data) {
            Ok(stored) => Some(stored),
            Err(error) => {
                tracing::warn!(id, %error, "an assistant conversation doesn't read");
                None
            }
        }
    }

    /// Saves `stored`, replacing its file in one step.
    pub fn save(&self, stored: &Stored) -> Result<()> {
        let id = &stored.conversation.id;
        let path = self
            .file(id)
            .ok_or_else(|| color_eyre::eyre::eyre!("“{id}” is not a conversation id"))?;
        crate::app_paths::create_private_dir(&self.dir)
            .wrap_err("couldn't create the conversations folder")?;
        let data = serde_json::to_vec_pretty(stored)?;
        write_private(&path, &data).wrap_err("couldn't save the conversation")
    }

    /// Every conversation that reads, newest first.
    pub fn list(&self) -> Vec<ConversationSummary> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut list: Vec<_> = entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                Some(summary(&self.load(id)?))
            })
            .collect();
        list.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(a.id.cmp(&b.id)));
        list
    }

    /// Marks every conversation left `Running` by a Hex that is gone as
    /// `Interrupted` (at startup); their ids.
    pub fn recover_interrupted(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut recovered = Vec::new();
        for entry in entries.flatten() {
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .map(str::to_owned)
            else {
                continue;
            };
            let Some(mut stored) = self.load(&id) else {
                continue;
            };
            if stored.outcome == Outcome::Running
                && stored.current_outcome() == Outcome::Interrupted
            {
                stored.set_outcome(Outcome::Interrupted);
                if self.save(&stored).is_ok() {
                    recovered.push(id);
                }
            }
        }
        recovered
    }

    /// Records how a task on `id` ended when its run didn't (it was
    /// stopped while waiting out a limit, or after being steered): only a
    /// conversation still `Running` changes. When the assistant hasn't
    /// replied, `message` is added as its error so the transcript says why.
    pub fn settle(&self, id: &str, outcome: Outcome, message: Option<&str>) {
        let Some(mut stored) = self.load(id) else {
            return;
        };
        if stored.outcome != Outcome::Running {
            return;
        }
        let replied = stored
            .conversation
            .messages
            .last()
            .is_some_and(|last| last.role == Role::Assistant);
        if !replied
            && !stored.conversation.messages.is_empty()
            && let Some(message) = message
        {
            stored.push(Message {
                role: Role::Assistant,
                parts: vec![Part::Error {
                    message: message.to_owned(),
                }],
                at_ms: now_ms(),
            });
        }
        stored.set_outcome(outcome);
        if let Err(error) = self.save(&stored) {
            tracing::warn!(id, %error, "couldn't record how the task ended");
        }
    }

    /// Removes a conversation and its attachments.
    pub fn delete(&self, id: &str) {
        let Some(file) = self.file(id) else {
            return;
        };
        let _ = fs::remove_file(file);
        let _ = fs::remove_dir_all(self.dir.join(id));
    }

    /// Saves `png` beside conversation `id` and returns its file name.
    pub fn save_attachment(&self, id: &str, png: &[u8]) -> Result<String> {
        let folder = self.attachments(id)?;
        crate::app_paths::create_private_dir(&folder)
            .wrap_err("couldn't create the attachments folder")?;
        let name = format!("screenshot-{}.png", now_ms());
        write_private(&folder.join(&name), png).wrap_err("couldn't save the screenshot")?;
        Ok(name)
    }

    /// Where conversation `id`'s attachment `file` lives (it may not exist).
    pub fn attachment_path(&self, id: &str, file: &str) -> Option<PathBuf> {
        valid_file(file).then(|| self.attachments(id).ok().map(|dir| dir.join(file)))?
    }

    fn attachments(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            color_eyre::eyre::bail!("“{id}” is not a conversation id");
        }
        Ok(self.dir.join(id))
    }
}

/// `<support dir>/assistant`, created private to this user: conversations,
/// and the folder the agents run in.
pub fn assistant_dir() -> Result<PathBuf> {
    let dir = crate::app_paths::support_dir()?.join("assistant");
    crate::app_paths::create_private_dir(&dir)
        .wrap_err("couldn't create the assistant's folder")?;
    Ok(dir)
}

/// Where a conversation's attachment `file` lives.
pub fn attachment_path(conversation: &str, file: &str) -> Option<PathBuf> {
    Store::open().ok()?.attachment_path(conversation, file)
}

/// A new conversation id: 32 random hex digits in UUID form.
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or_default();
        bytes = (nanos ^ u128::from(std::process::id()) << 64).to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Unix milliseconds now.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis() as u64)
        .unwrap_or_default()
}

/// A title from the first question: its first line, cut at a word to about
/// 60 characters.
pub fn title(question: &str) -> String {
    let line = question
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("New conversation");
    shorten(line, TITLE_CHARS)
}

/// `text` on one line, cut at a word to at most `limit` characters with an
/// ellipsis when it was longer.
pub fn shorten(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > limit / 2 => &cut[..space],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end_matches([' ', ',', '.', ';', ':']))
}

/// Whether process `pid` still runs.
fn alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    crate::assistant::guard::process_alive(pid)
}

fn summary(stored: &Stored) -> ConversationSummary {
    let conversation = &stored.conversation;
    let preview = conversation
        .messages
        .iter()
        .rev()
        .map(Message::text)
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default();
    ConversationSummary {
        id: conversation.id.clone(),
        title: conversation.title.clone(),
        updated_ms: conversation.updated_ms,
        origin: conversation.origin,
        preview: shorten(&preview, PREVIEW_CHARS),
        outcome: stored.current_outcome(),
    }
}

/// Whether `id` is safe as a file name: letters, digits, `-` and `_`.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether `file` names one file in an attachments folder.
fn valid_file(file: &str) -> bool {
    !file.is_empty()
        && !file.starts_with('.')
        && file.len() <= 128
        && file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Writes `data` to a private temporary file beside `path`, then renames it
/// over `path`.
pub(crate) fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", new_id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::types::{Part, Role};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_store() -> Store {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        Store::at(std::env::temp_dir().join(format!(
            "hex-assistant-store-{}-{id}-{}",
            std::process::id(),
            now_ms()
        )))
    }

    fn message(role: Role, text: &str, at_ms: u64) -> Message {
        Message {
            role,
            parts: vec![Part::Text { text: text.into() }],
            at_ms,
        }
    }

    #[test]
    fn a_conversation_round_trips_with_its_sessions_and_attachments() {
        let store = temp_store();
        let id = new_id();
        let mut stored = Stored::new(&id, Origin::Chat, "Wie spät ist es in Tokio?");
        let file = store.save_attachment(&id, b"\x89PNG fake").unwrap();
        stored.push(Message {
            role: Role::User,
            parts: vec![
                Part::Text {
                    text: "Wie spät ist es in Tokio?".into(),
                },
                Part::Screenshot { file: file.clone() },
            ],
            at_ms: 10,
        });
        stored.push(Message {
            role: Role::Assistant,
            parts: vec![
                Part::Action {
                    description: "Opened Clock".into(),
                },
                Part::Text {
                    text: "Es ist 18 Uhr.".into(),
                },
            ],
            at_ms: u64::MAX / 2,
        });
        stored
            .sessions
            .insert("claude_code".into(), "session-1".into());
        store.save(&stored).unwrap();

        assert_eq!(store.load(&id), Some(stored.clone()));
        let path = store.attachment_path(&id, &file).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"\x89PNG fake");
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Wie spät ist es in Tokio?");
        assert_eq!(list[0].preview, "Es ist 18 Uhr.");
        assert_eq!(list[0].updated_ms, u64::MAX / 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.file(&id).unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }

        store.delete(&id);
        assert_eq!(store.load(&id), None);
        assert!(!path.exists());
        assert!(store.list().is_empty());
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn the_list_is_newest_first_and_skips_unreadable_files() {
        let store = temp_store();
        for (id, at) in [("a", 5), ("b", 50), ("c", 20)] {
            let mut stored = Stored::new(id, Origin::Background, id);
            stored.push(message(Role::User, id, at));
            stored.conversation.updated_ms = at;
            store.save(&stored).unwrap();
        }
        fs::write(store.dir.join("broken.json"), b"{").unwrap();
        let ids: Vec<_> = store.list().into_iter().map(|s| s.id).collect();
        let _ = fs::remove_dir_all(&store.dir);
        assert_eq!(ids, ["b", "c", "a"]);
    }

    #[test]
    fn a_conversation_left_running_by_a_gone_process_reads_as_interrupted() {
        let store = temp_store();
        let mut running = Stored::new("live", Origin::Background, "live");
        running.push(message(Role::User, "live", 1));
        running.set_running();
        store.save(&running).unwrap();
        let mut gone = Stored::new("gone", Origin::Background, "gone");
        gone.push(message(Role::User, "gone", 2));
        gone.set_running();
        // No process has pid 0x7fff_fff0 on a Mac.
        gone.pid = Some(0x7fff_fff0);
        store.save(&gone).unwrap();
        let mut done = Stored::new("done", Origin::Chat, "done");
        done.set_outcome(Outcome::Failed);
        store.save(&done).unwrap();

        let outcomes = |store: &Store| -> Vec<(String, Outcome)> {
            let mut list: Vec<_> = store
                .list()
                .into_iter()
                .map(|s| (s.id, s.outcome))
                .collect();
            list.sort_by(|a, b| a.0.cmp(&b.0));
            list
        };
        assert_eq!(
            outcomes(&store),
            [
                ("done".into(), Outcome::Failed),
                ("gone".into(), Outcome::Interrupted),
                ("live".into(), Outcome::Running),
            ]
        );
        assert_eq!(store.recover_interrupted(), ["gone"]);
        let gone = store.load("gone").unwrap();
        assert_eq!((gone.outcome, gone.pid), (Outcome::Interrupted, None));
        assert_eq!(gone.last_question().as_deref(), Some("gone"));
        assert!(store.recover_interrupted().is_empty());

        store.settle("live", Outcome::Cancelled, Some("Stopped."));
        // Only a conversation still running is settled.
        store.settle("done", Outcome::Cancelled, Some("Stopped."));
        assert_eq!(store.load("done").unwrap().outcome, Outcome::Failed);
        let live = store.load("live").unwrap();
        assert_eq!(live.outcome, Outcome::Cancelled);
        assert_eq!(
            live.conversation.messages.last().unwrap().parts,
            [Part::Error {
                message: "Stopped.".into()
            }]
        );
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn older_files_without_an_outcome_read_as_answered() {
        let stored: Stored = serde_json::from_str(
            r#"{"id":"x","title":"t","created_ms":1,"updated_ms":1,"origin":"agent_mode","messages":[]}"#,
        )
        .unwrap();
        assert_eq!(stored.outcome, Outcome::Answered);
        assert_eq!(stored.conversation.origin, Origin::Background);
    }

    #[test]
    fn titles_are_cut_at_a_word() {
        assert_eq!(title("  \n Short one \nsecond"), "Short one");
        let long =
            "Please find the invoice from last March in Mail and forward it to the accountant";
        let cut = title(long);
        assert!(cut.chars().count() <= 60, "{cut}");
        assert_eq!(cut, "Please find the invoice from last March in Mail and…");
    }

    #[test]
    fn ids_and_attachment_names_cannot_leave_the_folder() {
        let store = temp_store();
        assert!(store.attachment_path("../x", "a.png").is_none());
        assert!(store.attachment_path("abc", "../a.png").is_none());
        assert!(store.attachment_path("abc", ".hidden").is_none());
        assert!(store.attachment_path("abc", "a.png").is_some());
        assert!(
            store
                .save(&Stored::new("../evil", Origin::Chat, "x"))
                .is_err()
        );
        let id = new_id();
        assert_eq!(id.len(), 36);
        assert!(valid_id(&id));
        assert_ne!(id, new_id());
    }
}
