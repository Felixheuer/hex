//! An app's scripting dictionary (`sdef <app>`), read with a small tag
//! scanner and summarized to the useful core within a budget, and a
//! `whose` search example from its classes. Pure.

use super::*;

/// One class (or class extension) of a dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Class {
    name: String,
    inherits: Option<String>,
    properties: Vec<String>,
    elements: Vec<String>,
}

/// One command of a dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Command {
    name: String,
    direct: Option<String>,
    /// (name, optional)
    parameters: Vec<(String, bool)>,
}

/// One suite of a dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Suite {
    name: String,
    classes: Vec<Class>,
    commands: Vec<Command>,
}

/// A dictionary, as far as a summary needs it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Dictionary {
    suites: Vec<Suite>,
    /// It has (or includes) the standard suite: make, delete, count, …
    standard: bool,
    /// The standard suites' own classes (window, document, …): shown only
    /// where they hold the app's own classes (a browser's windows hold its
    /// tabs).
    standard_classes: Vec<Class>,
}

/// One tag of the XML.
pub(super) struct Tag<'a> {
    name: &'a str,
    attrs: &'a str,
    /// A start tag (or an empty one).
    opens: bool,
    /// An end tag (or an empty one).
    closes: bool,
}

/// The tags of `xml` in order; comments, declarations, and text skipped.
pub(super) fn tags(xml: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        let skip_to = |rest: &str, end: &str| rest.find(end).map(|at| at + end.len());
        if rest.starts_with("<!--") {
            let Some(end) = skip_to(rest, "-->") else {
                break;
            };
            rest = &rest[end..];
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            let Some(end) = skip_to(rest, ">") else { break };
            rest = &rest[end..];
            continue;
        }
        // The tag's end: the first `>` outside quotes.
        let mut quote = None;
        let mut end = None;
        for (at, c) in rest.char_indices().skip(1) {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '>') => {
                    end = Some(at);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        let inner = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Tag {
                name: name.trim(),
                attrs: "",
                opens: false,
                closes: true,
            });
            continue;
        }
        let empty = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let (name, attrs) = match inner.find(char::is_whitespace) {
            Some(at) => (&inner[..at], &inner[at..]),
            None => (inner, ""),
        };
        out.push(Tag {
            name,
            attrs,
            opens: true,
            closes: empty,
        });
    }
    out
}

/// Attribute `key` of a tag, its entities decoded.
pub(super) fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let close = after[1..].find(quote)? + 1;
        if name == key {
            return Some(decode(&after[1..close]));
        }
        rest = &after[close + 1..];
    }
}

pub(super) fn decode(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

pub(super) fn hidden(attrs: &str) -> bool {
    attr(attrs, "hidden").is_some_and(|value| value == "yes")
}

/// Suites every scriptable app has (make, delete, count, …; text): named
/// once in a summary instead of listed.
pub(super) fn standard_suite(name: &str, code: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("standard suite")
        || name == "text suite"
        || name == "type definitions"
        || matches!(code, "core" | "????" | "TEXT")
}

/// The dictionary in `sdef`'s XML.
pub(super) fn parse(xml: &str) -> Dictionary {
    let mut dictionary = Dictionary::default();
    let mut suite: Option<Suite> = None;
    let mut class: Option<Class> = None;
    let mut command: Option<Command> = None;
    let mut in_direct = false;
    // Inside a standard suite: its classes are kept apart, its commands
    // named once.
    let mut standard = false;
    // How deep inside a hidden item or a suite left out we are.
    let mut skipping = 0usize;
    let mut stack: Vec<&str> = Vec::new();
    for tag in tags(xml) {
        if tag.opens {
            if skipping > 0 {
                if !tag.closes {
                    skipping += 1;
                    stack.push(tag.name);
                }
                continue;
            }
            match tag.name {
                "xi:include" => {
                    if attr(tag.attrs, "href").is_some_and(|href| href.contains("CocoaStandard")) {
                        dictionary.standard = true;
                    }
                }
                "suite" => {
                    let name = attr(tag.attrs, "name").unwrap_or_default();
                    let code = attr(tag.attrs, "code").unwrap_or_default();
                    if hidden(tag.attrs) {
                        if !tag.closes {
                            skipping = 1;
                            stack.push(tag.name);
                        }
                        continue;
                    }
                    standard = standard_suite(&name, &code);
                    dictionary.standard |= standard;
                    suite = Some(Suite {
                        name,
                        ..Suite::default()
                    });
                }
                "class" | "class-extension" | "command" | "property" | "element" | "parameter"
                    if hidden(tag.attrs) =>
                {
                    if !tag.closes {
                        skipping = 1;
                        stack.push(tag.name);
                    }
                    continue;
                }
                "class" | "class-extension" => {
                    let name = attr(tag.attrs, "name")
                        .or_else(|| attr(tag.attrs, "extends"))
                        .unwrap_or_default();
                    class = Some(Class {
                        name,
                        inherits: attr(tag.attrs, "inherits"),
                        ..Class::default()
                    });
                }
                "property" => {
                    if let (Some(class), Some(name)) = (class.as_mut(), attr(tag.attrs, "name")) {
                        class.properties.push(name);
                    }
                }
                "element" => {
                    if let (Some(class), Some(kind)) = (class.as_mut(), attr(tag.attrs, "type")) {
                        class.elements.push(kind);
                    }
                }
                "command" => {
                    command = Some(Command {
                        name: attr(tag.attrs, "name").unwrap_or_default(),
                        ..Command::default()
                    });
                }
                "direct-parameter" => {
                    if let Some(command) = command.as_mut() {
                        command.direct = attr(tag.attrs, "type");
                        in_direct = !tag.closes;
                    }
                }
                "type" if in_direct => {
                    if let (Some(command), Some(kind)) = (command.as_mut(), attr(tag.attrs, "type"))
                    {
                        match &mut command.direct {
                            Some(direct) => {
                                direct.push('|');
                                direct.push_str(&kind);
                            }
                            None => command.direct = Some(kind),
                        }
                    }
                }
                "parameter" => {
                    if let (Some(command), Some(name)) = (command.as_mut(), attr(tag.attrs, "name"))
                    {
                        let optional =
                            attr(tag.attrs, "optional").is_some_and(|value| value == "yes");
                        command.parameters.push((name, optional));
                    }
                }
                _ => {}
            }
            if !tag.closes {
                stack.push(tag.name);
            } else {
                close(
                    tag.name,
                    &mut suite,
                    &mut class,
                    &mut command,
                    &mut dictionary,
                    standard,
                );
            }
            continue;
        }
        // An end tag.
        stack.pop();
        if skipping > 0 {
            skipping -= 1;
            continue;
        }
        if tag.name == "direct-parameter" {
            in_direct = false;
        }
        close(
            tag.name,
            &mut suite,
            &mut class,
            &mut command,
            &mut dictionary,
            standard,
        );
    }
    dictionary
}

pub(super) fn close(
    name: &str,
    suite: &mut Option<Suite>,
    class: &mut Option<Class>,
    command: &mut Option<Command>,
    dictionary: &mut Dictionary,
    standard: bool,
) {
    match name {
        "class" | "class-extension" => {
            if let (Some(suite), Some(class)) = (suite.as_mut(), class.take()) {
                suite.classes.push(class);
            }
        }
        "command" => {
            if let (Some(suite), Some(command)) = (suite.as_mut(), command.take()) {
                suite.commands.push(command);
            }
        }
        "suite" => match suite.take() {
            Some(suite) if standard => dictionary.standard_classes.extend(suite.classes),
            Some(suite) if !suite.classes.is_empty() || !suite.commands.is_empty() => {
                dictionary.suites.push(suite)
            }
            _ => {}
        },
        _ => {}
    }
}

/// The standard classes that hold the app's own classes (a browser's
/// windows and their tabs), as a suite of their own; `None` when none do.
pub(super) fn holders(dictionary: &Dictionary) -> Option<Suite> {
    let own: std::collections::HashSet<&str> = dictionary
        .suites
        .iter()
        .flat_map(|suite| suite.classes.iter())
        .map(|class| class.name.as_str())
        .filter(|name| *name != "application")
        .collect();
    let classes: Vec<Class> = dictionary
        .standard_classes
        .iter()
        .filter(|class| {
            class
                .elements
                .iter()
                .any(|element| own.contains(element.as_str()))
        })
        .cloned()
        .collect();
    (!classes.is_empty()).then(|| Suite {
        name: "Standard classes holding the app's own".into(),
        classes,
        commands: Vec::new(),
    })
}

/// `dictionary` as lines, each class showing at most `cap` properties.
pub(super) fn render(dictionary: &Dictionary, cap: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let holders = holders(dictionary);
    for suite in dictionary.suites.iter().chain(holders.as_ref()) {
        lines.push(format!("{}:", suite.name));
        // The application's own properties and elements first: where every
        // script starts.
        let mut classes: Vec<&Class> = suite.classes.iter().collect();
        classes.sort_by_key(|class| class.name != "application");
        for class in classes {
            let mut line = format!("  {}", class.name);
            if let Some(parent) = &class.inherits {
                line.push_str(&format!(" (a {parent})"));
            }
            line.push(':');
            let mut parts = Vec::new();
            if !class.elements.is_empty() {
                parts.push(format!(" elements {}", class.elements.join(", ")));
            }
            if !class.properties.is_empty() {
                let mut shown: Vec<&str> = class
                    .properties
                    .iter()
                    .take(cap)
                    .map(String::as_str)
                    .collect();
                let more = class.properties.len().saturating_sub(cap);
                let more = if more > 0 {
                    format!(" +{more}")
                } else {
                    String::new()
                };
                shown.dedup();
                parts.push(format!(" {}{more}", shown.join(", ")));
            }
            line.push_str(&parts.join(";"));
            lines.push(line);
        }
        if !suite.commands.is_empty() {
            let commands: Vec<String> = suite
                .commands
                .iter()
                .map(|command| {
                    let mut params: Vec<String> = command.direct.iter().cloned().collect();
                    params.extend(command.parameters.iter().map(|(name, optional)| {
                        if *optional {
                            format!("{name}?")
                        } else {
                            name.clone()
                        }
                    }));
                    if params.is_empty() {
                        command.name.clone()
                    } else {
                        format!("{}({})", command.name, params.join("; "))
                    }
                })
                .collect();
            lines.push(format!("  commands: {}", commands.join(", ")));
        }
    }
    lines
}

/// A compact summary of a scripting dictionary (`sdef`'s XML) within
/// `budget` characters; `None` when it defines nothing beyond the standard
/// suites. Fewer properties per class are shown until it fits; what still
/// doesn't is cut with a note.
pub fn summarize_sdef(xml: &str, budget: usize) -> Option<String> {
    let dictionary = parse(xml);
    if dictionary.suites.is_empty() {
        return None;
    }
    let standard = if dictionary.standard {
        "Plus the standard commands: make, delete, count, exists, move, duplicate, open, close, save.\n"
    } else {
        ""
    };
    let budget = budget.saturating_sub(standard.len());
    let mut lines = Vec::new();
    for cap in [16, 10, 6, 3] {
        lines = render(&dictionary, cap);
        if joined_len(&lines) <= budget {
            return Some(
                format!("{}\n{standard}", lines.join("\n"))
                    .trim_end()
                    .to_owned(),
            );
        }
    }
    const CUT: &str = "… (cut; the full dictionary: `sdef <app path>` in your shell)";
    let mut kept = Vec::new();
    let mut used = CUT.len() + 1;
    for line in lines {
        if used + line.len() + 1 > budget {
            break;
        }
        used += line.len() + 1;
        kept.push(line);
    }
    kept.push(CUT.to_owned());
    Some(
        format!("{}\n{standard}", kept.join("\n"))
            .trim_end()
            .to_owned(),
    )
}

pub(super) fn joined_len(lines: &[String]) -> usize {
    lines.iter().map(|line| line.len() + 1).sum()
}

/// Properties that name an object, in the order a search prefers them.
pub(super) const NAMING: &[&str] = &["name", "title", "subject"];

/// A search a script can start from, as `every <class> whose <name>
/// contains "…"`: among the application's elements, the class most others
/// hold (the app's central kind of thing: notes, tracks, mailboxes) that
/// has a naming property, own or inherited; the one with the most
/// properties on a tie.
pub fn search_example(xml: &str) -> Option<String> {
    let dictionary = parse(xml);
    let all: Vec<&Class> = dictionary
        .suites
        .iter()
        .flat_map(|suite| suite.classes.iter())
        .chain(dictionary.standard_classes.iter())
        .collect();
    let by_name: HashMap<&str, &Class> = all
        .iter()
        .filter(|class| class.name != "application")
        .map(|class| (class.name.as_str(), *class))
        .collect();
    // Its properties with those it inherits (a bounded chain).
    let properties = |class: &Class| -> Vec<String> {
        let mut out = class.properties.clone();
        let mut parent = class.inherits.as_deref();
        for _ in 0..8 {
            let Some(found) = parent.and_then(|name| by_name.get(name)) else {
                break;
            };
            out.extend(found.properties.iter().cloned());
            parent = found.inherits.as_deref();
        }
        out
    };
    let held = |name: &str| {
        all.iter()
            .filter(|class| class.elements.iter().any(|element| element == name))
            .count()
    };
    let mut app_elements: Vec<&str> = all
        .iter()
        .filter(|class| class.name == "application")
        .flat_map(|class| class.elements.iter().map(String::as_str))
        .collect();
    app_elements.dedup();
    let (class, naming, _) = app_elements
        .into_iter()
        .filter_map(|element| {
            let class = by_name.get(element)?;
            let properties = properties(class);
            let naming = NAMING
                .iter()
                .find(|naming| properties.iter().any(|property| property == *naming))?;
            Some((
                class.name.clone(),
                *naming,
                (held(element), properties.len()),
            ))
        })
        .max_by_key(|(_, _, rank)| *rank)?;
    Some(format!(
        "get {naming} of every {class} whose {naming} contains \"…\""
    ))
}
