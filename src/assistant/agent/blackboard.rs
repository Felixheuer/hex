//! A team's shared blackboard: `team.md` in the lead's folder, read by
//! every helper before it starts and written as it goes, so no two helpers
//! do the same item. Each line is one note:
//!
//! ```text
//! - claim: nike.com · 3f2a9c1e        a helper takes an item before doing it
//! - done: nike.com · 3f2a9c1e · rows.csv (12 rows)
//! - release: adidas.com · 3f2a9c1e    it gives an item back undone
//! - found: the shop lists prices without VAT · 3f2a9c1e
//! ```
//!
//! Claims go through `hex_team_board` ([`claim`]), which holds a lock file
//! while it reads and appends, so two helpers claiming the same item at the
//! same moment get one yes and one "taken". Items are compared loosely
//! ([`key`]): "https://www.Nike.com/" and "nike.com" are one item. Agents
//! may read the file directly; the lines stay plain Markdown.
//!
//! OWNER: brain.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// The board's file in the lead's folder.
pub const BOARD: &str = "team.md";
/// How long a claim waits for another writer's lock.
const LOCK_WAIT: Duration = Duration::from_secs(3);
/// A lock older than this was left by a writer that died.
const LOCK_STALE: Duration = Duration::from_secs(15);
/// The longest item, finding, or result kept.
const TEXT_KEPT: usize = 300;

const HEADER: &str = "# Team board\n\nShared by the job and its helpers. Read it before you start and before each new item. Claim an item before you do it (hex_team_board action claim); an item someone else claimed or finished is theirs. Mark it done with where its result is; give it back (release) if you stop. Note findings that help the others (found).\n\n";

/// The board for a task working in `workspace`: a lead's is in its own
/// folder, a helper's in its lead's (the folder around its own).
pub fn path_for(workspace: &Path, helper: bool) -> PathBuf {
    if helper && let Some(lead) = workspace.parent() {
        return lead.join(BOARD);
    }
    workspace.join(BOARD)
}

/// One line of the board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    Claim {
        item: String,
        by: String,
    },
    Done {
        item: String,
        by: String,
        result: String,
    },
    Release {
        item: String,
        by: String,
    },
    Found {
        text: String,
        by: String,
    },
}

impl Note {
    fn line(&self) -> String {
        match self {
            Self::Claim { item, by } => format!("- claim: {} · {by}\n", one_line(item)),
            Self::Done { item, by, result } if result.trim().is_empty() => {
                format!("- done: {} · {by}\n", one_line(item))
            }
            Self::Done { item, by, result } => {
                format!("- done: {} · {by} · {}\n", one_line(item), one_line(result))
            }
            Self::Release { item, by } => format!("- release: {} · {by}\n", one_line(item)),
            Self::Found { text, by } => format!("- found: {} · {by}\n", one_line(text)),
        }
    }

    fn parse(line: &str) -> Option<Self> {
        let line = line.trim_start().strip_prefix(['-', '*'])?.trim();
        let (word, rest) = line.split_once(':')?;
        let parts: Vec<&str> = rest.split(" · ").map(str::trim).collect();
        let first = (*parts.first()?).to_owned();
        let by = parts.get(1).map(|by| (*by).to_owned()).unwrap_or_default();
        if first.is_empty() {
            return None;
        }
        Some(match word.trim() {
            "claim" => Self::Claim { item: first, by },
            "done" => Self::Done {
                item: first,
                by,
                result: parts
                    .get(2..)
                    .map(|rest| rest.join(" · "))
                    .unwrap_or_default(),
            },
            "release" => Self::Release { item: first, by },
            "found" => Self::Found { text: first, by },
            _ => return None,
        })
    }
}

/// Who holds an item, and whether it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub item: String,
    pub by: String,
    pub done: bool,
    pub result: String,
}

/// The board as it stands: each item's holder (by [`key`]), and the
/// findings, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Board {
    pub items: BTreeMap<String, Held>,
    pub found: Vec<(String, String)>,
}

impl Board {
    pub fn parse(text: &str) -> Self {
        let mut board = Self::default();
        for note in text.lines().filter_map(Note::parse) {
            match note {
                Note::Claim { item, by } => {
                    let key = key(&item);
                    // A claim on an item someone holds changes nothing.
                    board.items.entry(key).or_insert(Held {
                        item,
                        by,
                        done: false,
                        result: String::new(),
                    });
                }
                Note::Done { item, by, result } => {
                    let key = key(&item);
                    let held = board.items.entry(key).or_insert_with(|| Held {
                        item: item.clone(),
                        by: by.clone(),
                        done: false,
                        result: String::new(),
                    });
                    if held.by == by || !held.done {
                        held.done = true;
                        held.by = by;
                        held.result = result;
                    }
                }
                Note::Release { item, by } => {
                    let key = key(&item);
                    if board
                        .items
                        .get(&key)
                        .is_some_and(|held| held.by == by && !held.done)
                    {
                        board.items.remove(&key);
                    }
                }
                Note::Found { text, by } => board.found.push((text, by)),
            }
        }
        board
    }

    /// Items claimed and not yet done, and items done.
    pub fn counts(&self) -> (usize, usize) {
        let done = self.items.values().filter(|held| held.done).count();
        (self.items.len() - done, done)
    }

    /// Items claimed (open) and done by `by`.
    pub fn counts_of(&self, by: &str) -> (usize, usize) {
        let mine = self.items.values().filter(|held| same_member(&held.by, by));
        let (mut open, mut done) = (0, 0);
        for held in mine {
            if held.done {
                done += 1;
            } else {
                open += 1;
            }
        }
        (open, done)
    }

    /// What a helper reads: who does what, what is done, and the findings.
    pub fn summary(&self, me: Option<&str>) -> String {
        if self.items.is_empty() && self.found.is_empty() {
            return "The board is empty: nobody has claimed anything yet.".into();
        }
        let (open, done) = self.counts();
        let mut out = format!("{open} item(s) claimed and in progress, {done} done.\n");
        let mut taken: Vec<String> = Vec::new();
        let mut finished: Vec<String> = Vec::new();
        for held in self.items.values() {
            let who = if me.is_some_and(|me| same_member(&held.by, me)) {
                "you".to_owned()
            } else {
                held.by.clone()
            };
            if held.done {
                let result = if held.result.is_empty() {
                    String::new()
                } else {
                    format!(" → {}", held.result)
                };
                finished.push(format!("- {} ({who}){result}", held.item));
            } else {
                taken.push(format!("- {} ({who})", held.item));
            }
        }
        if !taken.is_empty() {
            out.push_str("\nIn progress (don't do these):\n");
            out.push_str(&taken.join("\n"));
            out.push('\n');
        }
        if !finished.is_empty() {
            out.push_str("\nDone (don't redo these):\n");
            out.push_str(&finished.join("\n"));
            out.push('\n');
        }
        if !self.found.is_empty() {
            out.push_str("\nFindings:\n");
            for (text, by) in &self.found {
                out.push_str(&format!("- {text} ({by})\n"));
            }
        }
        out
    }
}

/// `by` as a member names itself on the board: a helper's id's first
/// eight characters (enough to tell 32 apart, short to read).
pub fn member_name(id: &str) -> String {
    id.chars().take(8).collect()
}

fn same_member(written: &str, id: &str) -> bool {
    let (a, b) = (written.trim(), id.trim());
    !a.is_empty() && !b.is_empty() && (a == b || b.starts_with(a) || a.starts_with(b))
}

/// An item as it is compared: lowercase, without a web address's scheme,
/// `www.`, or trailing slash, and with its spaces collapsed.
pub fn key(item: &str) -> String {
    let lower = item.trim().to_lowercase();
    let mut text = lower.as_str();
    for prefix in ["https://", "http://"] {
        text = text.strip_prefix(prefix).unwrap_or(text);
    }
    text = text.strip_prefix("www.").unwrap_or(text);
    text = text.trim_end_matches('/');
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn one_line(text: &str) -> String {
    let line = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" · ", " - ");
    crate::assistant::store::shorten(&line, TEXT_KEPT)
}

/// The board at `path` (empty when there is none).
pub fn read(path: &Path) -> Board {
    Board::parse(&std::fs::read_to_string(path).unwrap_or_default())
}

/// What a claim found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claimed {
    /// It's yours now (or already was).
    Yours,
    /// Someone else holds it, or finished it.
    Taken(Held),
}

/// Claims `item` for `by`: yours unless another member holds or finished it.
pub fn claim(path: &Path, item: &str, by: &str) -> Result<Claimed, String> {
    if key(item).is_empty() {
        return Err("Name the item to claim (a site, a supplier, a file, a part).".into());
    }
    locked(path, |text| {
        let board = Board::parse(text);
        match board.items.get(&key(item)) {
            Some(held) if !same_member(&held.by, by) || held.done => {
                Ok((Claimed::Taken(held.clone()), None))
            }
            Some(_) => Ok((Claimed::Yours, None)),
            None => Ok((
                Claimed::Yours,
                Some(Note::Claim {
                    item: item.trim().to_owned(),
                    by: by.to_owned(),
                }),
            )),
        }
    })
}

/// Adds `note` to the board (done, release, or found).
pub fn add(path: &Path, note: Note) -> Result<(), String> {
    locked(path, |_| Ok(((), Some(note))))
}

/// Runs `decide` on the board's text while holding its lock, and appends
/// the note it returns.
fn locked<T>(
    path: &Path,
    decide: impl FnOnce(&str) -> Result<(T, Option<Note>), String>,
) -> Result<T, String> {
    let lock = path.with_extension("md.lock");
    let until = Instant::now() + LOCK_WAIT;
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&lock)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|at| SystemTime::now().duration_since(at).ok())
                    .is_some_and(|age| age > LOCK_STALE);
                if stale {
                    let _ = std::fs::remove_file(&lock);
                    continue;
                }
                if Instant::now() > until {
                    return Err("The team board is busy; try again in a moment.".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(format!("couldn't open the team board: {error}")),
        }
    }
    let result = (|| {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let (value, note) = decide(&text)?;
        if let Some(note) = note {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| format!("couldn't write the team board: {error}"))?;
            if text.is_empty() {
                file.write_all(HEADER.as_bytes())
                    .map_err(|error| error.to_string())?;
            }
            file.write_all(note.line().as_bytes())
                .map_err(|error| error.to_string())?;
        }
        Ok(value)
    })();
    let _ = std::fs::remove_file(&lock);
    result
}

/// Starts the board in a lead's `workspace` when it has none.
pub fn ensure(workspace: &Path) -> PathBuf {
    let path = workspace.join(BOARD);
    if !path.exists() {
        let _ = std::fs::write(&path, HEADER);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hex-board-{name}-{}-{}",
            std::process::id(),
            crate::assistant::store::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_claimed_item_is_nobody_else_s_until_released() {
        let dir = folder("claim");
        let board = ensure(&dir);
        assert_eq!(
            claim(&board, "https://www.Nike.com/", "aaaa1111").unwrap(),
            Claimed::Yours
        );
        // The same item, written differently, by another helper: taken.
        match claim(&board, "nike.com", "bbbb2222").unwrap() {
            Claimed::Taken(held) => assert_eq!(held.by, "aaaa1111"),
            other => panic!("{other:?}"),
        }
        // Claiming one's own again is fine, and writes nothing new.
        assert_eq!(
            claim(&board, "NIKE.COM", "aaaa1111").unwrap(),
            Claimed::Yours
        );
        let text = std::fs::read_to_string(&board).unwrap();
        assert_eq!(text.matches("- claim:").count(), 1, "{text}");
        // Released, another may take it.
        add(
            &board,
            Note::Release {
                item: "nike.com".into(),
                by: "aaaa1111".into(),
            },
        )
        .unwrap();
        assert_eq!(
            claim(&board, "nike.com", "bbbb2222").unwrap(),
            Claimed::Yours
        );
        // Done: nobody claims it again, not even its owner.
        add(
            &board,
            Note::Done {
                item: "nike.com".into(),
                by: "bbbb2222".into(),
                result: "rows.csv (12 rows)".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            claim(&board, "nike.com", "bbbb2222").unwrap(),
            Claimed::Taken(Held { done: true, .. })
        ));
        assert!(claim(&board, "  ", "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claims_racing_from_many_helpers_give_each_item_one_owner() {
        let dir = folder("race");
        let board = ensure(&dir);
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let board = board.clone();
                std::thread::spawn(move || {
                    (0..5)
                        .filter(|item| {
                            claim(&board, &format!("site{item}.com"), &format!("helper{n}"))
                                == Ok(Claimed::Yours)
                        })
                        .count()
                })
            })
            .collect();
        let won: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(won, 5, "every item went to exactly one helper");
        assert_eq!(read(&board).counts(), (5, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_board_counts_and_tells_who_does_what() {
        let text = "# Team board\n\n- claim: a.com · aaaa1111\n- claim: b.com · bbbb2222\n- done: a.com · aaaa1111 · a/rows.csv\n- found: prices are without VAT · bbbb2222\n- claim: c.com · aaaa1111\nnot a note\n";
        let board = Board::parse(text);
        assert_eq!(board.counts(), (2, 1));
        assert_eq!(board.counts_of("aaaa1111-full-id"), (1, 1));
        assert_eq!(board.counts_of("bbbb2222"), (1, 0));
        let said = board.summary(Some("aaaa1111ffff"));
        assert!(said.contains("- b.com (bbbb2222)"), "{said}");
        assert!(said.contains("- a.com (you) → a/rows.csv"), "{said}");
        assert!(said.contains("prices are without VAT"), "{said}");
        assert!(Board::default().summary(None).contains("empty"));
        assert_eq!(key(" HTTP://www.Shop.de/ "), "shop.de");
        assert_eq!(member_name("3f2a9c1e-77aa-4bbb"), "3f2a9c1e");
    }

    #[test]
    fn a_helper_s_board_is_in_its_lead_s_folder() {
        let lead = Path::new("/x/2026-11-13 Suppliers");
        assert_eq!(path_for(lead, false), lead.join(BOARD));
        assert_eq!(path_for(&lead.join("Nike"), true), lead.join(BOARD));
    }
}
