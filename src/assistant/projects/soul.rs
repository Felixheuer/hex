//! A project's soul, `soul.md`: who the project is, in Markdown the person
//! reads and edits — its goal, people with their roles and channels, sites,
//! repositories, accounts, tone and preferences, key files, decisions —
//! and a "Recently learned" section Agent-Desk adds to after tasks
//! ([`super::collect`]) and the person prunes.
//!
//! The file is plain sections under `## ` headings. Agent-Desk writes the
//! headings in the interface's language and recognises a section by its
//! heading in any language it speaks ([`Part::of`]); a section the person
//! adds is kept and read like any other.

use crate::assistant::knowledge::rank;

/// The sections a new soul starts with, in order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Goal,
    People,
    Sites,
    Repos,
    Accounts,
    Tone,
    Files,
    Decisions,
    Learned,
}

/// Interface languages a heading may be written in (`desktop::i18n`).
const LANGUAGES: [&str; 11] = [
    "en", "pl", "zh", "zh-Hant", "ja", "de", "es", "uk", "pt", "fr", "ru",
];

impl Part {
    pub const ALL: [Self; 9] = [
        Self::Goal,
        Self::People,
        Self::Sites,
        Self::Repos,
        Self::Accounts,
        Self::Tone,
        Self::Files,
        Self::Decisions,
        Self::Learned,
    ];

    /// The heading in English, the key of its translations.
    pub fn english(self) -> &'static str {
        match self {
            Self::Goal => "Goal",
            Self::People => "People",
            Self::Sites => "Websites",
            Self::Repos => "Repositories",
            Self::Accounts => "Accounts",
            Self::Tone => "Tone & preferences",
            Self::Files => "Key files",
            Self::Decisions => "Decisions",
            Self::Learned => "Recently learned",
        }
    }

    /// The name tools and the agent use (`hex_project`'s `section`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Goal => "goal",
            Self::People => "people",
            Self::Sites => "sites",
            Self::Repos => "repos",
            Self::Accounts => "accounts",
            Self::Tone => "tone",
            Self::Files => "files",
            Self::Decisions => "decisions",
            Self::Learned => "learned",
        }
    }

    /// The part a tool's `section` names (also "website", "repo", "person").
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_lowercase();
        let name = name.trim_end_matches('s');
        Self::ALL.into_iter().find(|part| {
            part.name().trim_end_matches('s') == name
                || matches!(
                    (part, name),
                    (Self::Sites, "website" | "site" | "url" | "link")
                        | (Self::Repos, "repository" | "repo" | "github")
                        | (Self::People, "person" | "people" | "contact" | "team")
                        | (Self::Accounts, "account")
                        | (Self::Tone, "tone" | "preference" | "style")
                        | (Self::Files, "file" | "document" | "data")
                        | (Self::Decisions, "decision")
                        | (Self::Goal, "goal" | "purpose" | "aim")
                )
        })
    }

    /// The heading in the interface's language.
    pub fn heading(self) -> &'static str {
        crate::desktop::i18n::tr(self.english())
    }

    /// The part a heading names, in any language.
    pub fn of(heading: &str) -> Option<Self> {
        let heading = rank::fold(heading.trim());
        Self::ALL.into_iter().find(|part| {
            LANGUAGES.iter().any(|language| {
                rank::fold(crate::desktop::i18n::tr_in(Some(language), part.english())) == heading
            })
        })
    }
}

/// One `## ` section: its heading and its lines as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub heading: String,
    pub lines: Vec<String>,
}

impl Section {
    /// Its lines with something in them (not blank, not a comment).
    fn items(&self) -> impl Iterator<Item = &str> {
        self.lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with("<!--"))
    }
}

/// A soul as its file holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Soul {
    /// The `# ` title (the project's name).
    pub title: String,
    /// What comes before the first section (the note about the file).
    pub intro: Vec<String>,
    pub sections: Vec<Section>,
}

/// The most lines "Recently learned" keeps; the oldest go first.
pub const LEARNED_MAX: usize = 60;

/// A new project's soul: its title, a note on what the file is, and the
/// sections, empty, headed in the interface's language.
pub fn template(name: &str) -> String {
    let mut text = format!("# {}\n\n", name.trim());
    text.push_str(&format!(
        "> {}\n",
        crate::desktop::i18n::tr(
            "This is the project's soul. Agent-Desk reads it for every task in the project and adds what it learns under “Recently learned”. Edit anything; delete what's wrong."
        )
    ));
    for part in Part::ALL {
        text.push_str(&format!("\n## {}\n\n", part.heading()));
    }
    text
}

/// `text` as sections.
pub fn parse(text: &str) -> Soul {
    let mut soul = Soul::default();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            soul.sections.push(Section {
                heading: heading.trim().to_owned(),
                lines: Vec::new(),
            });
        } else if let Some(title) = line.strip_prefix("# ")
            && soul.title.is_empty()
            && soul.sections.is_empty()
        {
            soul.title = title.trim().to_owned();
        } else if let Some(section) = soul.sections.last_mut() {
            section.lines.push(line.to_owned());
        } else {
            soul.intro.push(line.to_owned());
        }
    }
    soul
}

/// `soul` as its file.
pub fn render(soul: &Soul) -> String {
    let mut text = String::new();
    if !soul.title.is_empty() {
        text.push_str(&format!("# {}\n", soul.title));
    }
    let intro = trimmed(&soul.intro);
    if !intro.is_empty() {
        text.push('\n');
        for line in intro {
            text.push_str(line);
            text.push('\n');
        }
    }
    for section in &soul.sections {
        text.push_str(&format!("\n## {}\n\n", section.heading));
        let lines = trimmed(&section.lines);
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// `lines` without the blank ones at either end.
fn trimmed(lines: &[String]) -> &[String] {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(start, |end| end + 1);
    &lines[start..end]
}

impl Soul {
    /// The section of `part`, if the soul has one.
    pub fn section(&self, part: Part) -> Option<&Section> {
        self.sections
            .iter()
            .find(|section| Part::of(&section.heading) == Some(part))
    }

    fn section_mut(&mut self, part: Part) -> &mut Section {
        let at = match self
            .sections
            .iter()
            .position(|section| Part::of(&section.heading) == Some(part))
        {
            Some(at) => at,
            None => {
                // A new section goes before "Recently learned", which stays
                // last; that one goes at the end.
                let at = if part == Part::Learned {
                    self.sections.len()
                } else {
                    self.sections
                        .iter()
                        .position(|section| Part::of(&section.heading) == Some(Part::Learned))
                        .unwrap_or(self.sections.len())
                };
                self.sections.insert(
                    at,
                    Section {
                        heading: part.heading().to_owned(),
                        lines: Vec::new(),
                    },
                );
                at
            }
        };
        &mut self.sections[at]
    }

    /// Whether the soul says `value` anywhere already (folded, so case,
    /// accents, and a leading "www." don't matter).
    pub fn mentions(&self, value: &str) -> bool {
        let value = key(value);
        if value.is_empty() {
            return true;
        }
        format!(" {} ", key(&render(self))).contains(&format!(" {value} "))
    }

    /// Adds `line` as a list item under `part`; whether it was new.
    /// "Recently learned" keeps its newest [`LEARNED_MAX`] lines.
    pub fn add(&mut self, part: Part, line: &str) -> bool {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let line = line.trim_start_matches(['-', '*', ' ']).trim();
        if line.is_empty() {
            return false;
        }
        let new = key(line);
        if self
            .sections
            .iter()
            .flat_map(Section::items)
            .any(|item| key(item) == new)
        {
            return false;
        }
        let section = self.section_mut(part);
        // A section's lines end where its blank tail starts.
        while section
            .lines
            .last()
            .is_some_and(|last| last.trim().is_empty())
        {
            section.lines.pop();
        }
        section.lines.push(format!("- {line}"));
        if part == Part::Learned {
            let items: Vec<usize> = section
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.trim_start().starts_with("- "))
                .map(|(at, _)| at)
                .collect();
            if items.len() > LEARNED_MAX {
                let drop = items.len() - LEARNED_MAX;
                let gone: Vec<usize> = items.into_iter().take(drop).collect();
                let mut at = 0;
                section.lines.retain(|_| {
                    let keep = !gone.contains(&at);
                    at += 1;
                    keep
                });
            }
        }
        true
    }

    /// The soul in at most `budget` characters for a request's message:
    /// the lines that share words with `request` first, then each section
    /// in order ("Recently learned" newest first), shown in the file's
    /// order under their headings. Empty sections are left out.
    pub fn brief(&self, request: &str, budget: usize) -> String {
        let words: Vec<String> = rank::tokens(request)
            .into_iter()
            .filter(|word| word.chars().count() >= 4)
            .collect();
        // (section, line, priority): lower goes first.
        let mut items: Vec<(usize, usize, (u8, usize))> = Vec::new();
        for (at, section) in self.sections.iter().enumerate() {
            let learned = Part::of(&section.heading) == Some(Part::Learned);
            let lines: Vec<(usize, &str)> = section
                .lines
                .iter()
                .enumerate()
                .map(|(index, line)| (index, line.trim()))
                .filter(|(_, line)| !line.is_empty() && !line.starts_with("<!--"))
                .collect();
            let count = lines.len();
            for (order, (index, line)) in lines.into_iter().enumerate() {
                let folded = rank::fold(line);
                let hits = words
                    .iter()
                    .filter(|word| folded.contains(word.as_str()))
                    .count();
                let rank = if hits > 0 {
                    (0, usize::MAX - hits)
                } else if learned {
                    (2, count - order)
                } else {
                    (1, at * 1000 + order)
                };
                items.push((at, index, rank));
            }
        }
        items.sort_by_key(|(_, _, rank)| *rank);
        let mut used = 0;
        let mut kept: Vec<(usize, usize)> = Vec::new();
        for (at, index, _) in items {
            let line = self.sections[at].lines[index].trim();
            let cost = line.chars().count() + 1;
            let heading = if kept.iter().any(|(section, _)| *section == at) {
                0
            } else {
                self.sections[at].heading.chars().count() + 4
            };
            if used + cost + heading > budget {
                continue;
            }
            used += cost + heading;
            kept.push((at, index));
        }
        kept.sort_unstable();
        let mut text = String::new();
        let mut last = None;
        for (at, index) in kept {
            if last != Some(at) {
                text.push_str(&format!("## {}\n", self.sections[at].heading));
                last = Some(at);
            }
            text.push_str(self.sections[at].lines[index].trim());
            text.push('\n');
        }
        text
    }
}

/// `text` folded for comparing: lowercased, accents folded, without
/// "www.", a scheme, list marks, or the punctuation around words.
pub fn key(text: &str) -> String {
    let folded = rank::fold(text.trim());
    let folded = folded
        .trim_start_matches(['-', '*', ' '])
        .replace("https://", "")
        .replace("http://", "")
        .replace("www.", "");
    folded
        .split(|c: char| c.is_whitespace())
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
