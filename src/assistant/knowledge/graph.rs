//! The memory as a graph: people, companies, projects, and what reaches or
//! belongs to them (numbers, addresses, sites, repositories, folders), with
//! named relations ("Michael Schwarz —whatsapp→ +49…", "Kalibermarkt
//! —site→ staging.kalibermarkt.de —repo→ github.com/…"). Built without a
//! model from what the person had remembered (`hex_remember`, the memory
//! note) and from finished tasks, searchable (`hex_lookup` scope memory),
//! and the part a request mentions goes into its prompt ([`context`]).
//!
//! It lives beside the note at `<support>/assistant/memory-graph.json`,
//! private to this user. A finished task adds only sites, repositories, and
//! folders that carry the name of something the person already has in the
//! graph and named in the request: what a task read can't invent people or
//! relations, and nothing that looks like a secret is kept.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::rank;

/// Bounds that keep the file small.
const MAX_ENTITIES: usize = 2000;
const MAX_LINKS: usize = 6000;
/// How much of the graph a prompt carries.
const CONTEXT_CHARS: usize = 1200;
const CONTEXT_ENTITIES: usize = 6;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Person,
    Company,
    Project,
    Site,
    Repo,
    Folder,
    Phone,
    Email,
    Thing,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Company => "company",
            Self::Project => "project",
            Self::Site => "site",
            Self::Repo => "repo",
            Self::Folder => "folder",
            Self::Phone => "phone",
            Self::Email => "email",
            Self::Thing => "thing",
        }
    }

    /// A value (reached through a named thing) rather than a named thing.
    fn is_value(self) -> bool {
        matches!(
            self,
            Self::Site | Self::Repo | Self::Folder | Self::Phone | Self::Email
        )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entity {
    pub id: u64,
    pub kind: Kind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub updated_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Link {
    pub from: u64,
    pub rel: String,
    pub to: u64,
    /// "note", "remember", or "task:<id>".
    pub source: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Graph {
    pub entities: Vec<Entity>,
    pub links: Vec<Link>,
}

/// What one fact says: about whom, which values, and related names.
#[derive(Debug, Default, PartialEq)]
pub struct Fact {
    pub subject: Option<(String, Kind)>,
    /// (kind, relation, value)
    pub values: Vec<(Kind, String, String)>,
    /// (name, kind, relation) of another named thing.
    pub related: Vec<(String, Kind, String)>,
}

fn pattern(cell: &'static OnceLock<Regex>, source: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source).expect("valid pattern"))
}

fn emails(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(
        &RE,
        r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}",
    )
    .find_iter(text)
    .map(|found| found.as_str().to_lowercase())
    .collect()
}

fn repos(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(
        &RE,
        r"(?i)\b((?:github|gitlab|bitbucket)\.(?:com|org))/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)",
    )
    .captures_iter(text)
    .map(|found| {
        let repo = found[3].trim_end_matches(".git").trim_end_matches('.');
        format!("{}/{}/{repo}", found[1].to_lowercase(), &found[2])
    })
    .collect()
}

/// Web addresses (a host, with its path when one is given), not inside an
/// email or a repository link; bare hosts only with a common top-level
/// domain, so "z.B." or "main.rs" are none.
fn sites(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    const TLDS: [&str; 26] = [
        "de", "com", "io", "net", "org", "app", "dev", "ai", "eu", "at", "ch", "co", "uk", "info",
        "biz", "shop", "me", "tv", "so", "sh", "xyz", "cloud", "online", "store", "tech", "site",
    ];
    let re = pattern(
        &RE,
        r"(?i)(https?://)?\b((?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,})(/[^\s,;)\]>“”]*)?",
    );
    let mut found = Vec::new();
    for capture in re.captures_iter(text) {
        let whole = capture.get(0).expect("match");
        if text[..whole.start()].ends_with('@') || text[..whole.start()].ends_with('.') {
            continue;
        }
        let host = capture[2].to_lowercase();
        let explicit = capture.get(1).is_some();
        let tld = host.rsplit('.').next().unwrap_or_default();
        if !explicit && !TLDS.contains(&tld) {
            continue;
        }
        if host.starts_with("github.")
            || host.starts_with("gitlab.")
            || host.starts_with("bitbucket.")
        {
            continue;
        }
        let path = capture
            .get(3)
            .map(|path| path.as_str().trim_end_matches(['.', '/', ':']))
            .unwrap_or_default();
        let path = path.split(['?', '#']).next().unwrap_or_default();
        found.push(format!("{host}{path}"));
    }
    found
}

fn phones(text: &str, region: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(&RE, r"(?:\+|\b00|\b0)\d[\d \-/().]{5,}\d")
        .find_iter(text)
        .filter_map(|found| {
            let number = super::people::e164(found.as_str(), region);
            let digits = number.trim_start_matches('+');
            (number.starts_with('+')
                && (8..=15).contains(&digits.len())
                && digits.chars().all(|c| c.is_ascii_digit()))
            .then_some(number)
        })
        .collect()
}

fn folders(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(&RE, r#"(?:~|/Users/[^/\s]+)(?:/[^\s,;"'”)]+)+"#)
        .find_iter(text)
        .map(|found| found.as_str().trim_end_matches(['.', ':', '/']).to_owned())
        .collect()
}

/// Words that start with a capital but don't name anyone.
const NOT_NAMES: [&str; 64] = [
    "the",
    "my",
    "our",
    "his",
    "her",
    "their",
    "i",
    "a",
    "an",
    "mein",
    "meine",
    "meiner",
    "unser",
    "unsere",
    "sein",
    "seine",
    "ihr",
    "ihre",
    "ich",
    "die",
    "der",
    "das",
    "den",
    "dem",
    "ein",
    "eine",
    "er",
    "sie",
    "es",
    "wir",
    "telefonnummer",
    "nummer",
    "handynummer",
    "mobilnummer",
    "handy",
    "whatsapp",
    "signal",
    "telegram",
    "email",
    "e-mail",
    "mail",
    "website",
    "webseite",
    "seite",
    "site",
    "firma",
    "company",
    "kollege",
    "kollegin",
    "colleague",
    "chef",
    "boss",
    "kunde",
    "kundin",
    "customer",
    "projekt",
    "project",
    "repo",
    "ordner",
    "folder",
    "staging",
    "ton",
    "sprache",
    "phone",
];
const PARTICLES: [&str; 5] = ["von", "van", "de", "der", "zu"];
const COMPANY_SUFFIXES: [&str; 10] = [
    "gmbh", "ag", "ug", "kg", "se", "inc", "inc.", "ltd", "ltd.", "llc",
];

/// The first run of capitalized words that names someone or something
/// ("Michael Schwarz's WhatsApp …" → "Michael Schwarz"), at most four
/// words, a company's legal suffix kept.
pub fn leading_name(text: &str) -> Option<String> {
    let mut run: Vec<String> = Vec::new();
    for raw in text.split_whitespace() {
        let ends_run = raw.ends_with([',', ':', ';', '.', '!', '?'])
            && !raw.ends_with("Inc.")
            && !raw.ends_with("Ltd.");
        let word = raw
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '&' && c != '-' && c != '.')
            .trim_end_matches('.');
        let word = word
            .strip_suffix("'s")
            .or_else(|| word.strip_suffix("’s"))
            .unwrap_or(word);
        let possessive = raw.contains("'s") || raw.contains("’s");
        let lower = word.to_lowercase();
        let capital = word.chars().next().is_some_and(char::is_uppercase);
        let is_value =
            word.contains('@') || word.contains('/') || word.chars().any(|c| c.is_ascii_digit());
        let part_of_name = !run.is_empty()
            && (PARTICLES.contains(&lower.as_str()) || COMPANY_SUFFIXES.contains(&lower.as_str()));
        if (capital && !NOT_NAMES.contains(&lower.as_str()) && !is_value) || part_of_name {
            run.push(word.to_owned());
            if possessive || ends_run || run.len() == 4 {
                break;
            }
        } else if !run.is_empty() {
            break;
        }
    }
    while run
        .last()
        .is_some_and(|word| PARTICLES.contains(&word.to_lowercase().as_str()))
    {
        run.pop();
    }
    (!run.is_empty()).then(|| run.join(" "))
}

/// Another named thing the fact relates the subject to ("… bei
/// Kalibermarkt", "… works at Acme Inc").
fn workplaces(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(&RE, r"\b(?:bei|at|works for|arbeitet für|von der Firma|from)\s+(\p{Lu}[^\s,;:]*(?:\s+\p{Lu}[^\s,;:]*){0,3})")
        .captures_iter(text)
        .filter_map(|found| leading_name(&found[1]))
        .collect()
}

/// What `fact` says. `about`, when the agent named it, is the subject.
pub fn extract(fact: &str, about: Option<&str>, region: &str) -> Fact {
    let lower = fact.to_lowercase();
    let channel = if lower.contains("whatsapp") {
        "whatsapp"
    } else if lower.contains("signal") {
        "signal"
    } else if lower.contains("telegram") {
        "telegram"
    } else {
        "phone"
    };
    let mut values = Vec::new();
    for email in emails(fact) {
        values.push((Kind::Email, "email".to_owned(), email));
    }
    for phone in phones(fact, region) {
        values.push((Kind::Phone, channel.to_owned(), phone));
    }
    for repo in repos(fact) {
        values.push((Kind::Repo, "repo".to_owned(), repo));
    }
    let email_hosts: Vec<String> = values
        .iter()
        .filter(|(kind, _, _)| *kind == Kind::Email)
        .filter_map(|(_, _, email)| email.split_once('@').map(|(_, host)| host.to_owned()))
        .collect();
    for site in sites(fact) {
        if !email_hosts.contains(&site) {
            values.push((Kind::Site, "site".to_owned(), site));
        }
    }
    for folder in folders(fact) {
        values.push((Kind::Folder, "folder".to_owned(), folder));
    }
    values.dedup();
    let name = about
        .map(str::trim)
        .filter(|about| !about.is_empty())
        .map(str::to_owned)
        .or_else(|| leading_name(fact));
    let kind_of = |name: &str| {
        let words: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
        if words
            .iter()
            .any(|word| COMPANY_SUFFIXES.contains(&word.as_str()))
            || lower.contains("firma")
            || lower.contains("company")
        {
            Kind::Company
        } else if lower.contains("projekt") || lower.contains("project") {
            Kind::Project
        } else if values
            .iter()
            .any(|(kind, _, _)| matches!(kind, Kind::Phone | Kind::Email))
        {
            Kind::Person
        } else if values.iter().any(|(kind, _, _)| *kind == Kind::Site) {
            Kind::Company
        } else if values
            .iter()
            .any(|(kind, _, _)| matches!(kind, Kind::Repo | Kind::Folder))
        {
            Kind::Project
        } else {
            Kind::Thing
        }
    };
    let subject = name.map(|name| {
        let kind = kind_of(&name);
        (name, kind)
    });
    let related = workplaces(fact)
        .into_iter()
        .filter(|place| subject.as_ref().is_none_or(|(name, _)| name != place))
        .map(|place| (place, Kind::Company, "works at".to_owned()))
        .collect();
    Fact {
        subject,
        values,
        related,
    }
}

impl Graph {
    fn find(&self, kind: Option<Kind>, name: &str) -> Option<u64> {
        let key = rank::fold(name);
        self.entities
            .iter()
            .find(|entity| {
                kind.is_none_or(|kind| {
                    entity.kind == kind || (!kind.is_value() && !entity.kind.is_value())
                }) && (rank::fold(&entity.name) == key
                    || entity.aliases.iter().any(|alias| rank::fold(alias) == key))
            })
            .map(|entity| entity.id)
    }

    /// The entity named `name`, made when new; a more specific kind
    /// replaces "thing".
    fn upsert(&mut self, kind: Kind, name: &str, now_ms: u64) -> u64 {
        if let Some(id) = self.find(Some(kind), name) {
            if let Some(entity) = self.entities.iter_mut().find(|entity| entity.id == id) {
                if entity.kind == Kind::Thing {
                    entity.kind = kind;
                }
                entity.updated_ms = now_ms;
            }
            return id;
        }
        let id = self
            .entities
            .iter()
            .map(|entity| entity.id)
            .max()
            .unwrap_or(0)
            + 1;
        self.entities.push(Entity {
            id,
            kind,
            name: name.to_owned(),
            aliases: Vec::new(),
            updated_ms: now_ms,
        });
        id
    }

    fn link(&mut self, from: u64, rel: &str, to: u64, source: &str, now_ms: u64) -> bool {
        if from == to {
            return false;
        }
        if let Some(link) = self
            .links
            .iter_mut()
            .find(|link| link.from == from && link.to == to && link.rel == rel)
        {
            link.at_ms = now_ms;
            return false;
        }
        self.links.push(Link {
            from,
            rel: rel.to_owned(),
            to,
            source: source.to_owned(),
            at_ms: now_ms,
        });
        true
    }

    /// Adds what `fact` says; whether anything was new.
    pub fn learn(&mut self, fact: &Fact, source: &str, now_ms: u64) -> bool {
        let before = (self.entities.len(), self.links.len());
        let values: Vec<(u64, String, Kind)> = fact
            .values
            .iter()
            .map(|(kind, rel, value)| (self.upsert(*kind, value, now_ms), rel.clone(), *kind))
            .collect();
        match &fact.subject {
            Some((name, kind)) => {
                let subject = self.upsert(*kind, name, now_ms);
                for (value, rel, _) in &values {
                    self.link(subject, rel, *value, source, now_ms);
                }
                for (name, kind, rel) in &fact.related {
                    let other = self.upsert(*kind, name, now_ms);
                    self.link(subject, rel, other, source, now_ms);
                }
            }
            // No one named: the values belong together (a site and its
            // repository, a repository and its folder).
            None => {
                let mut values = values;
                values.sort_by_key(|(_, _, kind)| *kind);
                for pair in values.windows(2) {
                    let rel = pair[1].2.word();
                    self.link(pair[0].0, rel, pair[1].0, source, now_ms);
                }
            }
        }
        self.trim();
        (self.entities.len(), self.links.len()) != before
    }

    fn trim(&mut self) {
        if self.links.len() > MAX_LINKS {
            self.links.sort_by_key(|link| std::cmp::Reverse(link.at_ms));
            self.links.truncate(MAX_LINKS);
        }
        if self.entities.len() > MAX_ENTITIES {
            self.entities
                .sort_by_key(|entity| std::cmp::Reverse(entity.updated_ms));
            self.entities.truncate(MAX_ENTITIES);
            let kept: BTreeSet<u64> = self.entities.iter().map(|entity| entity.id).collect();
            self.links
                .retain(|link| kept.contains(&link.from) && kept.contains(&link.to));
        }
    }

    fn entity(&self, id: u64) -> Option<&Entity> {
        self.entities.iter().find(|entity| entity.id == id)
    }

    /// One line about `id`: its kind, what reaches it, what it belongs
    /// to, and one more step for a site's repository or a repository's
    /// folder.
    pub fn describe(&self, id: u64) -> Option<String> {
        let entity = self.entity(id)?;
        let mut parts = Vec::new();
        for link in self.links.iter().filter(|link| link.from == id) {
            let Some(to) = self.entity(link.to) else {
                continue;
            };
            let mut part = format!("{} {}", link.rel, to.name);
            if to.kind.is_value() {
                let further: Vec<String> = self
                    .links
                    .iter()
                    .filter(|next| next.from == to.id)
                    .filter_map(|next| {
                        self.entity(next.to)
                            .map(|other| format!("{} {}", next.rel, other.name))
                    })
                    .take(3)
                    .collect();
                if !further.is_empty() {
                    part.push_str(&format!(" ({})", further.join(", ")));
                }
            }
            if link.source.starts_with("task:") {
                part.push_str(" [from a task]");
            }
            parts.push(part);
        }
        for link in self.links.iter().filter(|link| link.to == id) {
            if let Some(from) = self.entity(link.from)
                && !from.kind.is_value()
            {
                parts.push(format!("{} ({})", from.name, link.rel));
            }
        }
        if parts.is_empty() && entity.kind.is_value() {
            return None;
        }
        Some(format!(
            "{} ({}){}{}",
            entity.name,
            entity.kind.word(),
            if parts.is_empty() { "" } else { ": " },
            parts.join("; ")
        ))
    }

    /// Entities matching `query` (by name, or by a value that reaches
    /// them), best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(f64, String)> {
        let tokens = rank::tokens(query);
        let mut scores: std::collections::BTreeMap<u64, f64> = Default::default();
        for entity in &self.entities {
            let text = format!("{} {}", entity.name, entity.aliases.join(" "));
            let score = rank::score(&text, &tokens);
            if score <= 0.0 {
                continue;
            }
            // A matching value counts for what it belongs to.
            let owners: Vec<u64> = if entity.kind.is_value() {
                self.links
                    .iter()
                    .filter(|link| link.to == entity.id)
                    .map(|link| link.from)
                    .collect()
            } else {
                Vec::new()
            };
            for id in owners.iter().copied().chain([entity.id]) {
                // What a value belongs to ranks above the bare value.
                let value = match (id == entity.id, entity.kind.is_value()) {
                    (true, true) => score * 0.8,
                    (true, false) => score,
                    (false, _) => score * 0.9,
                };
                let best = scores.entry(id).or_default();
                *best = best.max(value);
            }
        }
        let mut found: Vec<(f64, String)> = scores
            .into_iter()
            .filter_map(|(id, score)| self.describe(id).map(|line| (score, line)))
            .collect();
        found.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        found.dedup_by(|a, b| a.1 == b.1);
        found.truncate(limit);
        found
    }

    /// Named entities `text` mentions (a whole name, at least three
    /// letters), plus the owners of values it contains.
    fn mentioned(&self, text: &str) -> Vec<u64> {
        let folded = format!(" {} ", rank::fold(text));
        let spaced: String = folded
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '@' || c == '+' || c == '.' || c == '/' {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let mut ids = Vec::new();
        for entity in &self.entities {
            let names = std::iter::once(&entity.name).chain(entity.aliases.iter());
            let hit = names.into_iter().any(|name| {
                let name = rank::fold(name);
                name.chars().count() >= 3 && spaced.contains(&format!(" {name} "))
                    || (entity.kind.is_value() && name.len() >= 6 && folded.contains(&name))
            });
            if !hit {
                continue;
            }
            if entity.kind.is_value() {
                ids.extend(
                    self.links
                        .iter()
                        .filter(|link| link.to == entity.id)
                        .map(|link| link.from),
                );
            } else {
                ids.push(entity.id);
            }
        }
        let mut seen = BTreeSet::new();
        ids.retain(|id| seen.insert(*id));
        ids
    }

    /// The part of the graph a request mentions, for its prompt.
    pub fn context(&self, request: &str) -> Option<String> {
        let mut text = String::new();
        for id in self.mentioned(request).into_iter().take(CONTEXT_ENTITIES) {
            let Some(line) = self.describe(id) else {
                continue;
            };
            let line = crate::assistant::store::shorten(&line, 400);
            if text.chars().count() + line.chars().count() > CONTEXT_CHARS {
                break;
            }
            text.push_str("- ");
            text.push_str(&line);
            text.push('\n');
        }
        (!text.is_empty()).then(|| {
            format!("What you know about the people, companies, and projects this request mentions (memory graph):\n{text}")
        })
    }

    /// Links a finished task's sites, repositories, and folders to the
    /// things its `request` named, when they carry that thing's name
    /// ("Kalibermarkt" → staging.kalibermarkt.de); whether anything was new.
    pub fn learn_from_task(
        &mut self,
        task: &str,
        request: &str,
        answer: &str,
        now_ms: u64,
    ) -> bool {
        let named: Vec<(u64, String)> = self
            .mentioned(request)
            .into_iter()
            .filter_map(|id| {
                let entity = self.entity(id)?;
                let slug: String = rank::fold(&entity.name)
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect();
                (slug.len() >= 4).then_some((id, slug))
            })
            .collect();
        if named.is_empty() {
            return false;
        }
        let mut values: Vec<(Kind, String)> = Vec::new();
        values.extend(repos(answer).into_iter().map(|repo| (Kind::Repo, repo)));
        values.extend(sites(answer).into_iter().map(|site| (Kind::Site, site)));
        values.extend(
            folders(answer)
                .into_iter()
                .map(|folder| (Kind::Folder, folder)),
        );
        let source = format!("task:{}", task.chars().take(8).collect::<String>());
        let mut changed = false;
        for (kind, value) in values {
            if crate::assistant::memory::looks_secret(&value) || value.len() > 200 {
                continue;
            }
            let plain: String = rank::fold(&value)
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            for (id, slug) in &named {
                if plain.contains(slug.as_str()) {
                    let to = self.upsert(kind, &value, now_ms);
                    changed |= self.link(*id, kind.word(), to, &source, now_ms);
                }
            }
        }
        self.trim();
        changed
    }
}

/// `<support>/assistant/memory-graph.json`.
fn path() -> Option<PathBuf> {
    Some(
        crate::assistant::store::assistant_dir()
            .ok()?
            .join("memory-graph.json"),
    )
}

/// The graph at `path`; built from the memory `note` when there is none
/// yet (each line a fact).
pub fn load_from(path: &Path, note: impl FnOnce() -> String, region: &str) -> Graph {
    if let Ok(bytes) = std::fs::read(path)
        && let Ok(graph) = serde_json::from_slice::<Graph>(&bytes)
    {
        return graph;
    }
    let mut graph = Graph::default();
    let now = crate::assistant::store::now_ms();
    for line in note().lines() {
        let fact = line.trim().trim_start_matches("- ").trim();
        if !fact.is_empty() && !crate::assistant::memory::looks_secret(fact) {
            graph.learn(&extract(fact, None, region), "note", now);
        }
    }
    graph
}

pub fn save_to(path: &Path, graph: &Graph) -> Result<(), String> {
    let bytes = serde_json::to_vec(graph).map_err(|error| error.to_string())?;
    crate::assistant::store::write_private(path, &bytes).map_err(|error| error.to_string())
}

/// The person's graph.
pub fn load() -> Option<(PathBuf, Graph)> {
    let path = path()?;
    let region = crate::assistant::phone::incoming::region();
    let graph = load_from(&path, crate::assistant::memory::read, &region);
    Some((path, graph))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_found_and_normalized() {
        let fact = extract(
            "Michael Schwarz's WhatsApp is 0162 1234567, mail m.schwarz@kalibermarkt.de",
            None,
            "DE",
        );
        assert_eq!(fact.subject, Some(("Michael Schwarz".into(), Kind::Person)));
        assert_eq!(
            fact.values,
            [
                (
                    Kind::Email,
                    "email".into(),
                    "m.schwarz@kalibermarkt.de".into()
                ),
                (Kind::Phone, "whatsapp".into(), "+491621234567".into()),
            ],
            "the email's host is no site"
        );
        let site = extract(
            "Kalibermarkt staging is https://staging.kalibermarkt.de/shop?x=1, repo github.com/Felixheuer/kalibermarkt.git, checked out in ~/dev/kalibermarkt.",
            None,
            "DE",
        );
        assert_eq!(site.subject, Some(("Kalibermarkt".into(), Kind::Company)));
        assert_eq!(
            site.values,
            [
                (
                    Kind::Repo,
                    "repo".into(),
                    "github.com/Felixheuer/kalibermarkt".into()
                ),
                (
                    Kind::Site,
                    "site".into(),
                    "staging.kalibermarkt.de/shop".into()
                ),
                (Kind::Folder, "folder".into(), "~/dev/kalibermarkt".into()),
            ]
        );
        assert!(sites("z.B. main.rs und report.md am 2026-11-13").is_empty());
        assert!(phones("am 2026-11-13 um 14.30", "DE").is_empty());
    }

    #[test]
    fn names_are_read_from_the_start_of_a_fact() {
        assert_eq!(
            leading_name("Die Telefonnummer von Max Mustermann ist …").as_deref(),
            Some("Max Mustermann")
        );
        assert_eq!(
            leading_name("My colleague Anna Weber, anna@x.de").as_deref(),
            Some("Anna Weber")
        );
        assert_eq!(
            leading_name("Kalibermarkt GmbH sells ammunition").as_deref(),
            Some("Kalibermarkt GmbH")
        );
        assert_eq!(
            leading_name("Ludwig van Beethoven was here").as_deref(),
            Some("Ludwig van Beethoven")
        );
        assert_eq!(leading_name("tone: friendly, du"), None);
        let fact = extract(
            "Anna Weber arbeitet bei Kalibermarkt, anna@kalibermarkt.de",
            None,
            "DE",
        );
        assert_eq!(
            fact.related,
            [("Kalibermarkt".into(), Kind::Company, "works at".into())]
        );
        let about = extract("Prefers calls after 10", Some("Anna Weber"), "DE");
        assert_eq!(about.subject, Some(("Anna Weber".into(), Kind::Thing)));
    }

    #[test]
    fn the_graph_links_searches_and_gives_context() {
        let mut graph = Graph::default();
        let learn =
            |graph: &mut Graph, fact: &str| graph.learn(&extract(fact, None, "DE"), "remember", 1);
        assert!(learn(
            &mut graph,
            "Michael Schwarz: WhatsApp +49 162 1234567"
        ));
        assert!(
            !learn(&mut graph, "Michael Schwarz: WhatsApp 0162 1234567"),
            "nothing new"
        );
        learn(
            &mut graph,
            "Kalibermarkt staging is staging.kalibermarkt.de",
        );
        learn(
            &mut graph,
            "staging.kalibermarkt.de is built from github.com/felix/kalibermarkt",
        );
        learn(&mut graph, "Michael Schwarz works at Kalibermarkt");

        let found = graph.search("schwarz", 3);
        assert_eq!(
            found[0].1,
            "Michael Schwarz (person): whatsapp +491621234567; works at Kalibermarkt"
        );
        let by_number = graph.search("+491621234567", 3);
        assert!(
            by_number[0].1.starts_with("Michael Schwarz"),
            "{by_number:?}"
        );
        let company = graph.search("kalibermarkt", 3);
        assert!(
            company.iter().any(|(_, line)| line
                == "Kalibermarkt (company): site staging.kalibermarkt.de (repo github.com/felix/kalibermarkt); Michael Schwarz (works at)"),
            "{company:?}"
        );

        let context = graph
            .context("Schreib Michael Schwarz auf WhatsApp, dass staging geht")
            .unwrap();
        assert!(
            context.contains("- Michael Schwarz (person): whatsapp +491621234567"),
            "{context}"
        );
        assert!(!context.contains("- Kalibermarkt"), "{context}");
        assert_eq!(graph.context("Wie wird das Wetter?"), None);
    }

    #[test]
    fn a_finished_task_only_adds_what_carries_a_named_thing() {
        let mut graph = Graph::default();
        graph.learn(
            &extract("Kalibermarkt is our shop", None, "DE"),
            "remember",
            1,
        );
        let answer = "Deployed to https://staging.kalibermarkt.de and pushed github.com/felix/kalibermarkt-shop; see also https://evil.example.com and ~/dev/other";
        assert!(!graph.learn_from_task("abcdef123", "Wie wird das Wetter?", answer, 2));
        assert!(graph.learn_from_task("abcdef123", "Deploy Kalibermarkt to staging", answer, 2));
        let line = graph.search("kalibermarkt", 1)[0].1.clone();
        assert!(
            line.contains("site staging.kalibermarkt.de [from a task]"),
            "{line}"
        );
        assert!(
            line.contains("repo github.com/felix/kalibermarkt-shop"),
            "{line}"
        );
        assert!(!line.contains("evil") && !line.contains("other"), "{line}");
    }

    #[test]
    fn the_graph_starts_from_the_note_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("hex-graph-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory-graph.json");
        let graph = load_from(
            &path,
            || "- Anna Weber, anna@x.de\n- My password is hunter2 at x.de\n".into(),
            "DE",
        );
        assert_eq!(
            graph.search("anna", 1)[0].1,
            "Anna Weber (person): email anna@x.de"
        );
        assert!(
            graph
                .search("x.de", 5)
                .iter()
                .all(|(_, line)| !line.contains("hunter2"))
        );
        save_to(&path, &graph).unwrap();
        assert_eq!(load_from(&path, String::new, "DE"), graph);
        let _ = std::fs::remove_dir_all(dir);
    }
}
