//! App recipes (`hex_app_action`): an app's action by its fastest way, a
//! deep link, AppleScript, or its CLI, from data (`recipes.toml`, compiled
//! in). [`build`] turns a recipe and the agent's parameters into one
//! [`Command`] (pure: each parameter encoded for where it stands, so text
//! never breaks out of its URL, its script string, or its argument);
//! [`run`] runs it without bringing anything in front.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// The recipes, as shipped.
const SHIPPED: &str = include_str!("recipes.toml");

/// How a recipe runs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A URL opened in the background (`open -g`).
    Url,
    /// A file or folder shown in Finder (`open -R`).
    Reveal,
    /// AppleScript (`osascript`).
    Applescript,
    /// A program and its arguments, without a shell.
    Cli,
}

/// One recipe.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Recipe {
    pub app: String,
    pub intent: String,
    /// The app as it runs.
    pub name: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    pub summary: String,
    pub kind: Kind,
    pub template: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub params: Vec<String>,
    #[serde(default)]
    pub consequence: Option<crate::assistant::autonomy::Category>,
    /// It shows a window of the app.
    #[serde(default)]
    pub window: bool,
}

impl Recipe {
    /// Whether it shows a window of the app (taken for the task, parked).
    pub fn opens_window(&self) -> bool {
        self.window
    }

    /// Its parameters: (name, required).
    pub fn parameters(&self) -> Vec<(String, bool)> {
        self.params
            .iter()
            .map(|param| match param.strip_suffix('?') {
                Some(name) => (name.to_owned(), false),
                None => (param.clone(), true),
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct File {
    recipe: Vec<Recipe>,
}

/// What [`run`] runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Open(String),
    Reveal(String),
    Script(String),
    Program { program: String, args: Vec<String> },
}

/// Every recipe shipped.
pub fn all() -> &'static [Recipe] {
    static ALL: OnceLock<Vec<Recipe>> = OnceLock::new();
    ALL.get_or_init(|| parse(SHIPPED).unwrap_or_default())
}

/// The recipes in `text` (TOML).
pub fn parse(text: &str) -> Result<Vec<Recipe>, String> {
    toml::from_str::<File>(text)
        .map(|file| file.recipe)
        .map_err(|error| format!("recipes: {error}"))
}

/// The recipe for `app` and `intent` (an app compares as app names do).
pub fn find<'a>(recipes: &'a [Recipe], app: &str, intent: &str) -> Option<&'a Recipe> {
    let app = crate::assistant::guard::app_key(app);
    let intent = intent.trim().to_lowercase();
    recipes.iter().find(|recipe| {
        recipe.intent == intent
            && (crate::assistant::guard::app_key(&recipe.app) == app
                || crate::assistant::guard::app_key(&recipe.name) == app
                || recipe
                    .bundle_id
                    .as_deref()
                    .is_some_and(|id| crate::assistant::guard::app_key(id) == app))
    })
}

/// The recipes as the tool's description lists them.
pub fn catalogue(recipes: &[Recipe]) -> String {
    recipes
        .iter()
        .map(|recipe| {
            let params = recipe.params.join(", ");
            format!(
                "{} {}({params}): {}",
                recipe.app, recipe.intent, recipe.summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `recipe` with `values` as one command; why not when a required
/// parameter is missing or a value doesn't fit its filter.
pub fn build(recipe: &Recipe, values: &BTreeMap<String, String>) -> Result<Command, String> {
    for (name, required) in recipe.parameters() {
        if required
            && values
                .get(&name)
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!("{} {} needs {name}.", recipe.app, recipe.intent));
        }
    }
    let known: Vec<String> = recipe
        .parameters()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    if let Some(unknown) = values.keys().find(|key| !known.contains(key)) {
        return Err(format!(
            "{} {} has no parameter {unknown} (it takes: {}).",
            recipe.app,
            recipe.intent,
            known.join(", ")
        ));
    }
    match recipe.kind {
        Kind::Url => fill(&recipe.template, values, Place::Url).map(Command::Open),
        Kind::Reveal => fill(&recipe.template, values, Place::Raw).map(Command::Reveal),
        Kind::Applescript => {
            let mut script = fill(&recipe.template, values, Place::AppleScript)?;
            if recipe.template.contains("|date}") {
                script.push_str(HEX_DATE);
            }
            Ok(Command::Script(script.trim().to_owned()))
        }
        Kind::Cli => {
            let args = recipe
                .args
                .iter()
                .map(|arg| fill(arg, values, Place::Raw))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Command::Program {
                program: recipe.template.clone(),
                args,
            })
        }
    }
}

/// The intents every app has, beyond its recipes: the recipes are fast
/// paths, never a limit on which apps the agent can drive.
pub const GENERIC: &[&str] = &["open_url", "script"];

/// A generic action for any installed app (`name`, `bundle_id`):
/// `open_url` opens any URL with that app in the background (its own
/// scheme, a file, a web page); `script` runs AppleScript inside
/// `tell application id …` for any scriptable app. `None` when `intent`
/// isn't one of [`GENERIC`].
pub fn generic(
    name: &str,
    bundle_id: Option<&str>,
    intent: &str,
    values: &BTreeMap<String, String>,
) -> Option<Result<(Recipe, Command), String>> {
    let intent = intent.trim().to_lowercase();
    if !GENERIC.contains(&intent.as_str()) {
        return None;
    }
    let value = |key: &str| {
        values
            .get(key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let recipe = |kind: Kind, params: &[&str], window: bool| Recipe {
        app: name.to_owned(),
        intent: intent.clone(),
        name: name.to_owned(),
        bundle_id: bundle_id.map(str::to_owned),
        summary: String::new(),
        kind,
        template: String::new(),
        args: Vec::new(),
        params: params.iter().map(|param| (*param).to_owned()).collect(),
        consequence: None,
        window,
    };
    Some(match intent.as_str() {
        "open_url" => {
            let Some(url) = value("url") else {
                return Some(Err(format!("{name} open_url needs url.")));
            };
            if url.starts_with('-') || url.contains('\n') {
                return Some(Err(format!("“{url}” isn't a URL.")));
            }
            let mut args = vec!["-g".to_owned()];
            if let Some(id) = bundle_id {
                args.extend(["-b".to_owned(), id.to_owned()]);
            }
            args.push(url);
            Ok((
                recipe(Kind::Cli, &["url"], true),
                Command::Program {
                    program: "/usr/bin/open".into(),
                    args,
                },
            ))
        }
        _ => {
            let Some(script) = value("script") else {
                return Some(Err(format!("{name} script needs script (AppleScript).")));
            };
            let Some(id) = bundle_id else {
                return Some(Err(format!("{name} has no bundle id to script.")));
            };
            if script.to_lowercase().contains("do shell script") {
                return Some(Err(
                    "Shell commands don't go through app scripts; use your shell tool.".into(),
                ));
            }
            let mut recipe = recipe(Kind::Applescript, &["script"], false);
            // What the script says it does decides whether it needs a yes
            // ("send", "delete", …), as a button's label would.
            recipe.consequence = script
                .lines()
                .find_map(crate::assistant::autonomy::classify);
            let id = id.replace(['"', '\\'], "");
            Ok((
                recipe,
                Command::Script(format!("tell application id \"{id}\"\n{script}\nend tell")),
            ))
        }
    })
}

/// An AppleScript date from numbers, as `{p|date}` writes them.
const HEX_DATE: &str = "
on hexDate(y, m, d, h, mi)
	set t to current date
	set day of t to 1
	set year of t to y
	set month of t to m
	set day of t to d
	set time of t to (h * 3600 + mi * 60)
	return t
end hexDate
";

/// Where a value goes.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Url,
    AppleScript,
    Raw,
}

/// `template` with each `{name}` or `{name|filter}` replaced.
fn fill(template: &str, values: &BTreeMap<String, String>, place: Place) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}');
        let inner = close.map(|close| &after[..close]);
        let placeholder = inner.filter(|inner| {
            !inner.is_empty()
                && inner
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '|')
        });
        let (Some(inner), Some(close)) = (placeholder, close) else {
            // Not a placeholder (an AppleScript record): kept.
            out.push('{');
            rest = after;
            continue;
        };
        let (name, filter) = match inner.split_once('|') {
            Some((name, filter)) => (name, Some(filter)),
            None => (inner, None),
        };
        let value = values.get(name).map(String::as_str).unwrap_or_default();
        out.push_str(&encode(value, filter, place)?);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn encode(value: &str, filter: Option<&str>, place: Place) -> Result<String, String> {
    let value = value.trim();
    let filtered = match filter {
        None => value.to_owned(),
        Some("phone") => phone(value)?,
        Some("email") => {
            if value
                .chars()
                .any(|c| c.is_whitespace() || c == '?' || c == '&')
            {
                return Err(format!("“{value}” isn't an email address."));
            }
            return Ok(match place {
                Place::Url => percent(value, "@.+-_,"),
                _ => value.to_owned(),
            });
        }
        Some("date") => return date(value),
        Some("pane") => {
            let fits = !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-?=&".contains(c));
            if !fits {
                return Err(format!("“{value}” isn't a System Settings pane id."));
            }
            return Ok(value.to_owned());
        }
        Some("path") => path(value),
        Some("shell") => shell_quote(&path(value)),
        Some(other) => return Err(format!("unknown filter {other}")),
    };
    Ok(match place {
        Place::Url => percent(&filtered, ""),
        Place::AppleScript => applescript(&filtered),
        Place::Raw => filtered,
    })
}

/// A phone number as WhatsApp wants it: digits only, international, no
/// leading 00 or +.
fn phone(value: &str) -> Result<String, String> {
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.strip_prefix("00").unwrap_or(&digits).to_owned();
    if digits.len() < 7 {
        return Err(format!("“{value}” isn't a phone number."));
    }
    if digits.starts_with('0') {
        return Err(format!(
            "“{value}” is a national number; give it with its country code (4917… for Germany)."
        ));
    }
    Ok(digits)
}

/// "YYYY-MM-DD HH:MM" (or with T, or a date alone) as `y, m, d, h, mi`.
fn date(value: &str) -> Result<String, String> {
    let bad = || format!("“{value}” isn't a date as YYYY-MM-DD HH:MM.");
    let (day, time) = match value.split_once([' ', 'T']) {
        Some((day, time)) => (day, Some(time)),
        None => (value, None),
    };
    let mut parts = day.split('-').map(|part| part.parse::<u32>());
    let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let (h, mi) = match time {
        Some(time) => {
            let mut parts = time.split(':').map(|part| part.parse::<u32>());
            match (parts.next(), parts.next()) {
                (Some(Ok(h)), Some(Ok(mi))) => (h, mi),
                _ => return Err(bad()),
            }
        }
        None => (9, 0),
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || y < 1970 {
        return Err(bad());
    }
    Ok(format!("{y}, {m}, {d}, {h}, {mi}"))
}

fn path(value: &str) -> String {
    match value.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|home| home.join(rest).display().to_string())
            .unwrap_or_else(|| value.to_owned()),
        None => value.to_owned(),
    }
}

/// Percent-encodes all but unreserved characters (and `keep`); spaces as
/// %20, which every app reads.
fn percent(value: &str, keep: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        let c = byte as char;
        if byte.is_ascii_alphanumeric()
            || "-._~".contains(c)
            || (byte.is_ascii() && keep.contains(c))
        {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A value inside an AppleScript string literal.
fn applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// A value as one shell word.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Runs `command` without bringing anything in front; what it printed.
pub fn run(command: &Command) -> Result<String, String> {
    let output = match command {
        Command::Open(url) => std::process::Command::new("/usr/bin/open")
            .args(["-g", url])
            .output(),
        Command::Reveal(path) => std::process::Command::new("/usr/bin/open")
            .args(["-R", path])
            .output(),
        Command::Script(script) => {
            return super::recovery::run_osascript(script);
        }
        Command::Program { program, args } => {
            let program = resolve(program);
            std::process::Command::new(program).args(args).output()
        }
    }
    .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// A program by name: on `PATH` (as Hex finds tools), VS Code's `code` in
/// its bundle.
fn resolve(program: &str) -> std::path::PathBuf {
    if program.starts_with('/') {
        return program.into();
    }
    if let Some(found) = crate::assistant::backends::search::find(program) {
        return found;
    }
    if program == "code" {
        let bundled = std::path::Path::new(
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
        );
        if bundled.exists() {
            return bundled.into();
        }
    }
    program.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn recipe(app: &str, intent: &str) -> &'static Recipe {
        find(all(), app, intent).unwrap_or_else(|| panic!("no recipe {app} {intent}"))
    }

    #[test]
    fn every_shipped_recipe_parses_and_names_its_app() {
        let all = all();
        assert!(all.len() >= 20, "{}", all.len());
        for recipe in all {
            assert!(!recipe.summary.is_empty());
            assert!(!recipe.name.is_empty());
        }
        // Names compare as app names do: invisible marks, case.
        assert!(find(all, "\u{200e}WhatsApp", "open_chat").is_some());
        assert!(find(all, "net.whatsapp.WhatsApp", "OPEN_CHAT").is_some());
        assert!(find(all, "whatsapp", "send").is_none());
    }

    #[test]
    fn a_whatsapp_draft_is_a_deep_link_that_never_sends() {
        let whatsapp = recipe("whatsapp", "open_chat");
        assert_eq!(whatsapp.consequence, None);
        let built = build(
            whatsapp,
            &values(&[
                ("phone", "+49 176 123-45678"),
                ("text", "Hallo Michael & co? äö"),
            ]),
        );
        assert_eq!(
            built,
            Ok(Command::Open(
                "whatsapp://send?phone=4917612345678&text=Hallo%20Michael%20%26%20co%3F%20%C3%A4%C3%B6"
                    .into()
            ))
        );
        // 0049 counts as +49; a national number is refused.
        assert_eq!(
            build(whatsapp, &values(&[("phone", "0049 176 1234567")])),
            Ok(Command::Open(
                "whatsapp://send?phone=491761234567&text=".into()
            ))
        );
        assert!(build(whatsapp, &values(&[("phone", "0176 1234567")])).is_err());
        assert!(build(whatsapp, &values(&[("text", "hi")])).is_err());
        assert!(build(whatsapp, &values(&[("phone", "4917612345"), ("to", "x")])).is_err());
    }

    #[test]
    fn mail_slack_and_settings_links_are_encoded() {
        assert_eq!(
            build(
                recipe("mail", "compose"),
                &values(&[("to", "a.b+c@example.com"), ("subject", "Angebot 2/3")])
            ),
            Ok(Command::Open(
                "mailto:a.b+c@example.com?subject=Angebot%202%2F3&body=".into()
            ))
        );
        assert!(
            build(
                recipe("mail", "compose"),
                &values(&[("to", "x@y.z?bcc=evil")])
            )
            .is_err()
        );
        assert_eq!(
            build(
                recipe("slack", "open_channel"),
                &values(&[("team", "T123"), ("channel", "C456")])
            ),
            Ok(Command::Open("slack://channel?team=T123&id=C456".into()))
        );
        assert_eq!(
            build(
                recipe("system settings", "open_pane"),
                &values(&[(
                    "pane",
                    "com.apple.preference.security?Privacy_Accessibility"
                )])
            ),
            Ok(Command::Open(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
                    .into()
            ))
        );
        assert!(
            build(
                recipe("system settings", "open_pane"),
                &values(&[("pane", "x\"; do shell script \"rm")])
            )
            .is_err()
        );
    }

    #[test]
    fn script_values_never_break_out_of_their_string() {
        let Ok(Command::Script(script)) = build(
            recipe("notes", "create"),
            &values(&[("title", "Ein \"Zitat\" \\ hier"), ("body", "zwei\nZeilen")]),
        ) else {
            panic!("expected a script");
        };
        assert!(
            script.contains(r#"{name:"Ein \"Zitat\" \\ hier", body:"zwei"#),
            "{script}"
        );
        let Ok(Command::Script(script)) = build(
            recipe("calendar", "create_event"),
            &values(&[
                ("title", "Call"),
                ("start", "2026-11-14 09:30"),
                ("end", "2026-11-14T10:00"),
            ]),
        ) else {
            panic!("expected a script");
        };
        assert!(
            script.contains("start date:(my hexDate(2026, 11, 14, 9, 30))"),
            "{script}"
        );
        assert!(script.contains("on hexDate(y, m, d, h, mi)"));
        assert!(
            build(
                recipe("calendar", "create_event"),
                &values(&[
                    ("title", "x"),
                    ("start", "morgen"),
                    ("end", "2026-11-14 10:00")
                ])
            )
            .is_err()
        );
        let Ok(Command::Script(script)) = build(
            recipe("terminal", "claude"),
            &values(&[("dir", "/tmp/it's here")]),
        ) else {
            panic!("expected a script");
        };
        assert_eq!(
            script,
            r#"tell application id "com.apple.Terminal" to do script "cd '/tmp/it'\\''s here' && claude""#
        );
    }

    #[test]
    fn any_app_opens_urls_and_takes_scripts_without_a_recipe() {
        let (recipe, command) = generic(
            "Linear",
            Some("com.linear"),
            "open_url",
            &values(&[("url", "linear://issue/ABC-1")]),
        )
        .unwrap()
        .unwrap();
        assert!(recipe.opens_window());
        assert_eq!(
            command,
            Command::Program {
                program: "/usr/bin/open".into(),
                args: vec![
                    "-g".into(),
                    "-b".into(),
                    "com.linear".into(),
                    "linear://issue/ABC-1".into()
                ],
            }
        );
        assert!(
            generic("X", None, "open_url", &values(&[("url", "--args")]))
                .unwrap()
                .is_err()
        );
        let (recipe, command) = generic(
            "OmniFocus",
            Some("com.omnigroup.OmniFocus4"),
            "script",
            &values(&[("script", "get name of every flattened project")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(recipe.consequence, None);
        assert_eq!(
            command,
            Command::Script(
                "tell application id \"com.omnigroup.OmniFocus4\"\nget name of every flattened project\nend tell"
                    .into()
            )
        );
        // What a script does decides whether it needs a yes.
        let (recipe, _) = generic(
            "Mail",
            Some("com.apple.mail"),
            "script",
            &values(&[("script", "set m to first outgoing message\nsend m")]),
        )
        .unwrap()
        .unwrap();
        assert!(recipe.consequence.is_some());
        assert!(
            generic(
                "Finder",
                Some("com.apple.finder"),
                "script",
                &values(&[("script", "do shell script \"rm -rf ~\"")])
            )
            .unwrap()
            .is_err()
        );
        assert!(generic("X", None, "dance", &values(&[])).is_none());
    }

    #[test]
    fn sending_needs_consent_and_cli_recipes_take_plain_arguments() {
        assert_eq!(
            recipe("messages", "send").consequence,
            Some(crate::assistant::autonomy::Category::Send)
        );
        assert_eq!(
            build(
                recipe("shortcuts", "run"),
                &values(&[("shortcut", "Mein Kurzbefehl; rm -rf")])
            ),
            Ok(Command::Program {
                program: "/usr/bin/shortcuts".into(),
                args: vec!["run".into(), "Mein Kurzbefehl; rm -rf".into()],
            })
        );
        assert_eq!(
            build(
                recipe("finder", "reveal"),
                &values(&[("path", "/Users/x/a b.pdf")])
            ),
            Ok(Command::Reveal("/Users/x/a b.pdf".into()))
        );
    }
}
