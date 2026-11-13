//! `hex_app_capabilities`: what any installed app offers, found at runtime
//! from the app itself, never from a list of apps. From its bundle:
//!
//! - its scripting dictionary (`sdef <app>`), summarized to the useful
//!   core: per suite the classes with their properties and elements and
//!   the commands with their parameters, within [`DICTIONARY_BUDGET`]
//!   characters ([`summarize_sdef`], pure);
//! - the URL schemes it declares (`CFBundleURLTypes`) and the documents it
//!   opens or keeps (`CFBundleDocumentTypes`, `NSDocumentClass`, through
//!   [`super::recovery::traits_of`]);
//! - what its windows are made of (native, Electron, Catalyst, an iPhone or
//!   iPad app), which says how far Accessibility reaches;
//! - the link kinds it is the person's default for (LaunchServices: the
//!   browser for `https`, the mail app for `mailto`).
//!
//! [`describe`] reads all of it in about 25 ms and keeps it per app
//! version (its `Info.plist`) for the life of the process.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

/// Most characters of a dictionary summary.
pub const DICTIONARY_BUDGET: usize = 4000;

/// Most document type names listed.
const MAX_DOCUMENT_TYPES: usize = 8;

/// One class (or class extension) of a dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
struct Class {
    name: String,
    inherits: Option<String>,
    properties: Vec<String>,
    elements: Vec<String>,
}

/// One command of a dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
struct Command {
    name: String,
    direct: Option<String>,
    /// (name, optional)
    parameters: Vec<(String, bool)>,
}

/// One suite of a dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
struct Suite {
    name: String,
    classes: Vec<Class>,
    commands: Vec<Command>,
}

/// A dictionary, as far as a summary needs it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Dictionary {
    suites: Vec<Suite>,
    /// It has (or includes) the standard suite: make, delete, count, …
    standard: bool,
    /// The standard suites' own classes (window, document, …): shown only
    /// where they hold the app's own classes (a browser's windows hold its
    /// tabs).
    standard_classes: Vec<Class>,
}

/// One tag of the XML.
struct Tag<'a> {
    name: &'a str,
    attrs: &'a str,
    /// A start tag (or an empty one).
    opens: bool,
    /// An end tag (or an empty one).
    closes: bool,
}

/// The tags of `xml` in order; comments, declarations, and text skipped.
fn tags(xml: &str) -> Vec<Tag<'_>> {
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
fn attr(attrs: &str, key: &str) -> Option<String> {
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

fn decode(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn hidden(attrs: &str) -> bool {
    attr(attrs, "hidden").is_some_and(|value| value == "yes")
}

/// Suites every scriptable app has (make, delete, count, …; text): named
/// once in a summary instead of listed.
fn standard_suite(name: &str, code: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("standard suite")
        || name == "text suite"
        || name == "type definitions"
        || matches!(code, "core" | "????" | "TEXT")
}

/// The dictionary in `sdef`'s XML.
fn parse(xml: &str) -> Dictionary {
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

fn close(
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
fn holders(dictionary: &Dictionary) -> Option<Suite> {
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
fn render(dictionary: &Dictionary, cap: usize) -> Vec<String> {
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

fn joined_len(lines: &[String]) -> usize {
    lines.iter().map(|line| line.len() + 1).sum()
}

/// Properties that name an object, in the order a search prefers them.
const NAMING: &[&str] = &["name", "title", "subject"];

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

/// What a bundle's windows are made of.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ui {
    Native,
    Electron,
    Catalyst,
    /// An iPhone or iPad app running on the Mac.
    Mobile,
}

impl Ui {
    fn describe(self) -> &'static str {
        match self {
            Self::Native => {
                "native (AppKit): Accessibility reaches its controls, also in parked windows"
            }
            Self::Electron => {
                "Electron (a web page in a desktop frame): Accessibility sees less and acting \
                 in it is slow; prefer its URL schemes, its web version in your browser, or \
                 hex_read_window to read it"
            }
            Self::Catalyst => {
                "Catalyst (an iPad app made for the Mac): Accessibility reaches most controls; \
                 menus and scripting are thin"
            }
            Self::Mobile => {
                "an iPhone or iPad app on the Mac: Accessibility is thin; hex_read_window reads it"
            }
        }
    }
}

/// What an app's bundle declares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bundle {
    pub version: Option<String>,
    pub url_schemes: Vec<String>,
    pub document_types: Vec<String>,
    pub traits: super::recovery::Traits,
}

/// [`Bundle`] from an `Info.plist` (as JSON).
pub fn bundle_of(info: &Value) -> Bundle {
    let strings = |value: Option<&Value>| -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let mut url_schemes: Vec<String> = info
        .get("CFBundleURLTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|kind| strings(kind.get("CFBundleURLSchemes")))
        .map(|scheme| scheme.to_lowercase())
        .collect();
    url_schemes.sort();
    url_schemes.dedup();
    let mut document_types: Vec<String> = info
        .get("CFBundleDocumentTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|kind| {
            kind.get("CFBundleTypeName")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| strings(kind.get("LSItemContentTypes")).into_iter().next())
        })
        .collect();
    document_types.dedup();
    document_types.truncate(MAX_DOCUMENT_TYPES);
    let version = ["CFBundleShortVersionString", "CFBundleVersion"]
        .iter()
        .filter_map(|key| info.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    Bundle {
        version: (!version.is_empty()).then_some(version),
        url_schemes,
        document_types,
        traits: super::recovery::traits_of(info),
    }
}

/// [`Ui`] from an `Info.plist` and what the bundle holds.
pub fn ui_of(info: &Value, electron_framework: bool, wrapped: bool) -> Ui {
    if wrapped || info.get("LSRequiresIPhoneOS").and_then(Value::as_bool) == Some(true) {
        Ui::Mobile
    } else if electron_framework {
        Ui::Electron
    } else if info.get("UIDeviceFamily").is_some() {
        Ui::Catalyst
    } else {
        Ui::Native
    }
}

/// The URL schemes LaunchServices opens with `bundle_id` by default (from
/// its `LSHandlers`, as JSON): `https` for the person's browser, `mailto`
/// for their mail app, and any other.
pub fn default_schemes(launch_services: &Value, bundle_id: &str) -> Vec<String> {
    let mut schemes: Vec<String> = launch_services
        .get("LSHandlers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|handler| {
            handler
                .get("LSHandlerRoleAll")
                .and_then(Value::as_str)
                .is_some_and(|id| id.eq_ignore_ascii_case(bundle_id))
        })
        .filter_map(|handler| handler.get("LSHandlerURLScheme").and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect();
    schemes.sort();
    schemes.dedup();
    schemes
}

/// The person's LaunchServices choices (`LSHandlers`, as JSON): which app
/// opens which kind of link. `None` when never changed from the system's.
pub fn launch_services() -> Option<Value> {
    let path = dirs::home_dir()?
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    plist_json(&path)
}

fn plist_json(path: &Path) -> Option<Value> {
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// Everything [`describe`] says about an app, from what was read.
pub fn compose(
    name: &str,
    bundle_id: Option<&str>,
    info: &Value,
    sdef: Option<&str>,
    ui: Ui,
    default_for: &[String],
    tools: &[String],
) -> Value {
    let bundle = bundle_of(info);
    let dictionary = sdef.and_then(|xml| summarize_sdef(xml, DICTIONARY_BUDGET));
    let example = sdef.and_then(search_example);
    let scriptable = dictionary.is_some();
    let mut ways = Vec::new();
    if scriptable {
        ways.push(format!(
            "script: hex_app_action intent script (AppleScript, run inside `tell application id \
             \"{}\"`, without its window, on any desktop); read and change its data with the \
             dictionary below{}",
            bundle_id.unwrap_or(name),
            example
                .as_deref()
                .map(|example| format!(", e.g. `{example}`"))
                .unwrap_or_default()
        ));
    }
    if !bundle.url_schemes.is_empty() {
        ways.push(format!(
            "url: hex_app_action intent open_url with its schemes ({})",
            bundle
                .url_schemes
                .iter()
                .map(|scheme| format!("{scheme}:"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !tools.is_empty() {
        ways.push(format!(
            "cli: command-line tools it ships, for your shell: {}",
            tools.join(", ")
        ));
    }
    ways.push(format!(
        "window: hex_open_app, then act through Accessibility in its parked window ({})",
        ui.describe()
    ));
    ways.push("read: hex_read_window reads any window's text from its picture".into());
    json!({
        "app": name,
        "bundle_id": bundle_id,
        "version": bundle.version,
        "scripting": if scriptable {
            "AppleScript and JXA work (hex_app_action takes AppleScript)"
        } else if bundle.traits.scriptable {
            "only the standard suite (windows, documents)"
        } else {
            "not scriptable"
        },
        "dictionary": dictionary,
        "url_schemes": bundle.url_schemes,
        "document_types": bundle.document_types,
        "keeps_documents": bundle.traits.document_based,
        "ui": match ui {
            Ui::Native => "native",
            Ui::Electron => "electron",
            Ui::Catalyst => "catalyst",
            Ui::Mobile => "iphone-ipad",
        },
        "default_for": default_for,
        "command_line_tools": tools,
        "ways": ways,
    })
}

/// Where app bundles keep the command-line tools they ship.
const TOOL_DIRS: &[&str] = &["Contents/Resources/app/bin", "Contents/SharedSupport/bin"];

/// Most command-line tools listed.
const MAX_TOOLS: usize = 6;

/// The command-line tools the bundle at `path` ships ([`TOOL_DIRS`]).
fn tools_in(path: &Path) -> Vec<String> {
    let mut tools: Vec<String> = TOOL_DIRS
        .iter()
        .filter_map(|dir| std::fs::read_dir(path.join(dir)).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_dir()))
        .map(|entry| entry.path().display().to_string())
        .collect();
    tools.sort();
    tools.truncate(MAX_TOOLS);
    tools
}

/// What the installed app at `path` offers ([`compose`]); read once per
/// app version.
pub fn describe(name: &str, bundle_id: Option<&str>, path: &Path) -> Value {
    static CACHE: Mutex<Option<HashMap<PathBuf, (String, Value)>>> = Mutex::new(None);
    let wrapped = path.join("WrappedBundle").exists();
    let info_path = path.join("Contents/Info.plist");
    // An update rewrites the bundle's Info.plist (its version is in it).
    let stamp = format!(
        "{:?}",
        std::fs::metadata(&info_path)
            .map(|meta| (meta.modified().ok(), meta.len()))
            .ok()
    );
    if let Ok(cache) = CACHE.lock()
        && let Some((seen, value)) = cache.as_ref().and_then(|cache| cache.get(path))
        && *seen == stamp
    {
        return value.clone();
    }
    let info = plist_json(&info_path).unwrap_or(Value::Null);
    let sdef = std::process::Command::new("/usr/bin/sdef")
        .arg(path)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());
    let electron = path
        .join("Contents/Frameworks/Electron Framework.framework")
        .exists();
    let ui = ui_of(&info, electron, wrapped);
    let default_for = bundle_id
        .and_then(|id| launch_services().map(|services| default_schemes(&services, id)))
        .unwrap_or_default();
    let value = compose(
        name,
        bundle_id,
        &info,
        sdef.as_deref(),
        ui,
        &default_for,
        &tools_in(path),
    );
    if let Ok(mut cache) = CACHE.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(path.to_owned(), (stamp, value.clone()));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small dictionary shaped like a notes app's: the standard suite
    /// included, a class extension of the application, hidden items, a
    /// record type whose properties belong to no class.
    const NOTES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE dictionary SYSTEM "file://localhost/System/Library/DTDs/sdef.dtd">
<dictionary xmlns:xi="http://www.w3.org/2001/XInclude">
  <xi:include href="file://localhost/System/Library/ScriptingDefinitions/CocoaStandard.sdef" xpointer="xpointer(/dictionary/suite)"/>
  <!-- a comment with <tags> inside -->
  <suite name="Notes Suite" code="note" description="Terms &amp; events">
    <command name="show" code="noteshow" description="Show an object in the UI">
      <direct-parameter description="The object to be shown">
        <type type="folder"/>
        <type type="note"/>
      </direct-parameter>
      <parameter name="separately" code="sepa" type="boolean" optional="yes"/>
      <result description="The object shown."><type type="note"/></result>
    </command>
    <command name="secret" code="notesecr" hidden="yes">
      <parameter name="leak" code="leak" type="text"/>
    </command>
    <class-extension extends="application" description="the Notes application">
      <property name="selection" code="seln"><type type="note" list="yes"/></property>
      <element type="folder"><cocoa key="scriptingFolders"/></element>
      <element type="note"/>
    </class-extension>
    <class name="folder" code="cfol" description="a folder &gt; notes">
      <element type="note"/>
      <property name="name" code="pnam" type="text"/>
      <property name="id" code="ID  " type="text" access="r"/>
    </class>
    <class name="note" code="note">
      <property name="name" code="pnam" type="text" description="the name (normally the first line)"/>
      <property name="body" code="body" type="text"/>
      <property name="plaintext" code="text" type="text" access="r"/>
      <property name="internal" code="intr" type="text" hidden="yes"/>
      <property name="modification date" code="asmo" type="date" access="r"/>
    </class>
    <record-type name="options" code="opts">
      <property name="stray" code="stry" type="text"/>
    </record-type>
  </suite>
  <suite name="Standard Suite" code="????">
    <command name="delete" code="coredelo"/>
    <class name="window" code="cwin"><property name="bounds" code="pbnd" type="rectangle"/></class>
  </suite>
</dictionary>"#;

    #[test]
    fn a_dictionary_is_summarized_to_its_classes_and_commands() {
        let summary = summarize_sdef(NOTES, DICTIONARY_BUDGET).unwrap();
        assert_eq!(
            summary,
            "Notes Suite:\n  application: elements folder, note; selection\n  folder: elements note; name, id\n  note: name, body, plaintext, modification date\n  commands: show(folder|note; separately?)\nPlus the standard commands: make, delete, count, exists, move, duplicate, open, close, save."
        );
        // Hidden items, the standard suite's own classes, and a record
        // type's properties are left out.
        for left_out in ["secret", "leak", "internal", "window", "bounds", "stray"] {
            assert!(!summary.contains(left_out), "{left_out} in {summary}");
        }
        assert_eq!(
            search_example(NOTES).as_deref(),
            Some("get name of every note whose name contains \"…\"")
        );
    }

    /// A browser-shaped dictionary: its windows (standard) hold its tabs
    /// (its own), its tracks inherit their name.
    const BROWSER: &str = r#"<dictionary>
  <suite name="Standard Suite" code="core">
    <class name="application" code="capp"><element type="window"/></class>
    <class name="window" code="cwin">
      <element type="tab"/>
      <property name="name" code="pnam" type="text"/>
    </class>
    <class name="document" code="docu"><property name="path" code="ppth" type="text"/></class>
  </suite>
  <suite name="Browser Suite" code="BrSu">
    <class name="tab" code="CrTb">
      <property name="URL" code="URL " type="text"/>
      <property name="title" code="pnam" type="text"/>
    </class>
    <class-extension extends="application"><element type="bookmark folder"/></class-extension>
    <class name="item" code="itm "><property name="name" code="pnam" type="text"/></class>
    <class name="bookmark folder" code="CrBf" inherits="item">
      <element type="bookmark folder"/>
      <property name="index" code="indx" type="integer"/>
    </class>
    <command name="execute" code="CrSuExJa">
      <direct-parameter type="specifier"/>
      <parameter name="javascript" code="JaSc" type="text"/>
    </command>
  </suite>
</dictionary>"#;

    #[test]
    fn standard_classes_show_where_they_hold_the_app_s_own() {
        let summary = summarize_sdef(BROWSER, DICTIONARY_BUDGET).unwrap();
        assert!(
            summary
                .contains("Standard classes holding the app's own:\n  window: elements tab; name"),
            "{summary}"
        );
        assert!(!summary.contains("document"), "{summary}");
        assert!(summary.contains("commands: execute(specifier; javascript)"));
        // A bookmark folder is named through what it inherits.
        assert_eq!(
            search_example(BROWSER).as_deref(),
            Some("get name of every bookmark folder whose name contains \"…\"")
        );
    }

    #[test]
    fn a_large_dictionary_fits_its_budget() {
        let mut xml = String::from("<dictionary><suite name=\"Big\" code=\"bigg\">");
        for class in 0..60 {
            xml.push_str(&format!(
                "<class name=\"thing {class}\" code=\"t{class:03}\">"
            ));
            for property in 0..30 {
                xml.push_str(&format!(
                    "<property name=\"field {property}\" code=\"f{property:03}\" type=\"text\"/>"
                ));
            }
            xml.push_str("</class>");
        }
        xml.push_str("</suite></dictionary>");
        let summary = summarize_sdef(&xml, 1500).unwrap();
        assert!(summary.len() <= 1500, "{}", summary.len());
        assert!(summary.starts_with("Big:\n  thing 0: field 0, field 1, field 2 +27"));
        assert!(summary.ends_with("in your shell)"), "{summary}");
        // Only the standard suite: nothing worth summarizing.
        assert_eq!(
            summarize_sdef(
                "<dictionary><suite name=\"Standard Suite\" code=\"core\"><command name=\"open\" code=\"aevtodoc\"/></suite></dictionary>",
                DICTIONARY_BUDGET
            ),
            None
        );
        assert_eq!(summarize_sdef("not xml", DICTIONARY_BUDGET), None);
    }

    #[test]
    fn a_bundle_says_its_schemes_documents_and_toolkit() {
        let info = json!({
            "CFBundleShortVersionString": "4.12",
            "CFBundleVersion": "4120",
            "CFBundleURLTypes": [
                {"CFBundleURLName": "Chat", "CFBundleURLSchemes": ["chatapp", "ChatApp-Beta"]},
                {"CFBundleURLSchemes": ["chatapp"]}
            ],
            "CFBundleDocumentTypes": [
                {"CFBundleTypeName": "Chat export", "NSDocumentClass": "Doc"},
                {"LSItemContentTypes": ["public.plain-text"]}
            ],
        });
        let bundle = bundle_of(&info);
        assert_eq!(bundle.version.as_deref(), Some("4.12 4120"));
        assert_eq!(bundle.url_schemes, ["chatapp", "chatapp-beta"]);
        assert_eq!(bundle.document_types, ["Chat export", "public.plain-text"]);
        assert!(bundle.traits.document_based);
        assert_eq!(ui_of(&info, true, false), Ui::Electron);
        assert_eq!(
            ui_of(&json!({"UIDeviceFamily": [2, 6]}), false, false),
            Ui::Catalyst
        );
        assert_eq!(ui_of(&info, false, true), Ui::Mobile);
        assert_eq!(ui_of(&json!({}), false, false), Ui::Native);
    }

    #[test]
    fn launch_services_says_which_links_an_app_opens_by_default() {
        let services = json!({"LSHandlers": [
            {"LSHandlerURLScheme": "https", "LSHandlerRoleAll": "org.example.browser"},
            {"LSHandlerURLScheme": "http", "LSHandlerRoleAll": "Org.Example.Browser"},
            {"LSHandlerURLScheme": "mailto", "LSHandlerRoleAll": "org.example.mail"},
            {"LSHandlerContentType": "public.html", "LSHandlerRoleAll": "org.example.browser"}
        ]});
        assert_eq!(
            default_schemes(&services, "org.example.browser"),
            ["http", "https"]
        );
        assert!(default_schemes(&services, "org.example.other").is_empty());
    }

    #[test]
    fn the_answer_orders_the_ways_fastest_first() {
        let info = json!({"CFBundleURLTypes": [{"CFBundleURLSchemes": ["notes"]}]});
        let answer = compose(
            "Notes",
            Some("com.apple.Notes"),
            &info,
            Some(NOTES),
            Ui::Native,
            &[],
            &[],
        );
        let ways: Vec<&str> = answer["ways"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(ways[0].starts_with("script:"), "{ways:?}");
        assert!(ways[0].contains("every note whose name contains"));
        assert!(ways[1].starts_with("url:") && ways[1].contains("notes:"));
        assert!(ways[2].starts_with("window:"));
        assert!(ways[3].starts_with("read:"));
        assert!(answer["dictionary"].as_str().unwrap().contains("plaintext"));
        // An app without a dictionary or schemes: its window, then reading.
        let plain = compose("Viewer", None, &json!({}), None, Ui::Electron, &[], &[]);
        assert_eq!(plain["scripting"], "not scriptable");
        assert_eq!(plain["ways"].as_array().unwrap().len(), 2);
        // A command-line tool it ships comes before its window.
        let editor = compose(
            "Editor",
            None,
            &json!({}),
            None,
            Ui::Electron,
            &[],
            &["/Applications/Editor.app/Contents/Resources/app/bin/editor".into()],
        );
        assert!(editor["ways"][0].as_str().unwrap().starts_with("cli:"));
        assert_eq!(plain["ui"], "electron");
    }
}
