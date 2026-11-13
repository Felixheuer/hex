//! `hex_app_action`: an action in any installed app by its fastest way,
//! without clicking and without a list of apps. Two intents work for every
//! app: `open_url` (a URL the app takes: its own scheme, a file, a page,
//! opened in the background) and `script` (AppleScript inside
//! `tell application id …`, for any scriptable app). What an app offers
//! for either is found at runtime with `hex_app_capabilities`
//! ([`super::capabilities`]), never from a hand-written list.
//!
//! [`build`] turns the agent's call into one [`Command`] (pure, tested)
//! and says what it really reaches ([`vet`]): every app a script
//! addresses, and whether it runs code beyond the apps' dictionaries,
//! which the guard refuses outside YOLO. [`run`] runs it without bringing
//! anything in front.

pub mod vet;

use std::collections::BTreeMap;

/// The intents every app has.
pub const INTENTS: &[&str] = &["open_url", "script"];

/// What one call does, for the guard's checks.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    /// The app as it is installed.
    pub app: String,
    pub bundle_id: Option<String>,
    pub intent: String,
    /// The autonomy category it falls under, if any (from what a script
    /// says it does, as a button's label would).
    pub consequence: Option<crate::assistant::autonomy::Category>,
    /// It may show a window of the app (taken for the task, parked).
    pub opens_window: bool,
    /// The other apps a script addresses by name, bundle id, or path
    /// (`tell application "…"` after an `end tell`, too).
    pub targets: Vec<String>,
    /// What it runs beyond the apps' dictionaries (a shell, another
    /// script, a raw Apple event, an executable), if anything: only in
    /// YOLO (`vet`).
    pub runs_code: Option<String>,
}

/// What [`run`] runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Script(String),
    Program { program: String, args: Vec<String> },
}

/// The action `intent` with `values` for the installed app `name`
/// (`bundle_id`); `None` when `intent` isn't one of [`INTENTS`], why not
/// when a value is missing or unsafe.
pub fn build(
    name: &str,
    bundle_id: Option<&str>,
    intent: &str,
    values: &BTreeMap<String, String>,
) -> Option<Result<(Action, Command), String>> {
    let intent = intent.trim().to_lowercase();
    if !INTENTS.contains(&intent.as_str()) {
        return None;
    }
    let value = |key: &str| {
        values
            .get(key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let wanted = if intent == "open_url" {
        "url"
    } else {
        "script"
    };
    if let Some(unknown) = values.keys().find(|key| key.as_str() != wanted) {
        return Some(Err(format!(
            "{name} {intent} has no parameter {unknown} (it takes: {wanted})."
        )));
    }
    let action = |opens_window: bool| Action {
        app: name.to_owned(),
        bundle_id: bundle_id.map(str::to_owned),
        intent: intent.clone(),
        consequence: None,
        opens_window,
        targets: Vec::new(),
        runs_code: None,
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
            let mut action = action(true);
            action.runs_code = vet::url_risk(&url);
            args.push(url);
            Ok((
                action,
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
            let mut action = action(false);
            let (targets, computed) = vet::targets(&script);
            action.runs_code = vet::script_risk(&script, &targets).or_else(|| {
                computed.then(|| "addresses an app by a name computed while it runs".to_owned())
            });
            action.targets = targets
                .into_iter()
                .filter(|target| !target.eq_ignore_ascii_case(id) && target != name)
                .collect();
            // What the script says it does decides whether it needs a yes
            // ("send", "delete", …), as a button's label would.
            action.consequence = script
                .lines()
                .find_map(crate::assistant::autonomy::classify);
            let id = id.replace(['"', '\\'], "");
            Ok((
                action,
                Command::Script(format!("tell application id \"{id}\"\n{script}\nend tell")),
            ))
        }
    })
}

/// Runs `command` without bringing anything in front; what it printed.
pub fn run(command: &Command) -> Result<String, String> {
    let output = match command {
        Command::Script(script) => return super::recovery::run_osascript(script),
        Command::Program { program, args } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn any_app_opens_urls_in_the_background() {
        let (action, command) = build(
            "Linear",
            Some("com.linear"),
            "open_url",
            &values(&[("url", "linear://issue/ABC-1")]),
        )
        .unwrap()
        .unwrap();
        assert!(action.opens_window);
        assert_eq!(action.consequence, None);
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
            build("X", None, "open_url", &values(&[("url", "--args")]))
                .unwrap()
                .is_err()
        );
        assert!(build("X", None, "open_url", &values(&[])).unwrap().is_err());
        assert!(
            build(
                "X",
                None,
                "open_url",
                &values(&[("url", "x:"), ("to", "y")])
            )
            .unwrap()
            .is_err()
        );
    }

    #[test]
    fn any_scriptable_app_takes_a_script_and_its_words_decide_consent() {
        let (action, command) = build(
            "Notes",
            Some("com.apple.Notes"),
            "script",
            &values(&[(
                "script",
                "get plaintext of every note whose name contains \"Kraftstoffnotdienst\"",
            )]),
        )
        .unwrap()
        .unwrap();
        assert!(!action.opens_window);
        assert_eq!(action.consequence, None);
        assert_eq!(
            command,
            Command::Script(
                "tell application id \"com.apple.Notes\"\nget plaintext of every note whose name contains \"Kraftstoffnotdienst\"\nend tell"
                    .into()
            )
        );
        // What a script does decides whether it needs a yes.
        let (action, _) = build(
            "Mail",
            Some("com.apple.mail"),
            "script",
            &values(&[("script", "set m to first outgoing message\nsend m")]),
        )
        .unwrap()
        .unwrap();
        assert!(action.consequence.is_some());
        // Shell commands, other apps, and programs are named for the guard.
        let (action, _) = build(
            "Finder",
            Some("com.apple.finder"),
            "script",
            &values(&[("script", "do shell script \"rm -rf ~\"")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(action.runs_code.as_deref(), Some("runs shell commands"));
        let (action, _) = build(
            "Notes",
            Some("com.apple.Notes"),
            "script",
            &values(&[(
                "script",
                "get name of note 1\nend tell\ntell application \"1Password\" to get name\ntell application id \"com.apple.Notes\" to get name",
            )]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(action.targets, ["1Password"]);
        assert_eq!(action.runs_code, None);
        let (action, _) = build(
            "Notes",
            Some("com.apple.Notes"),
            "script",
            &values(&[(
                "script",
                "set t to \"Terminal\"\ntell application t to get name",
            )]),
        )
        .unwrap()
        .unwrap();
        assert!(action.runs_code.unwrap().contains("computed"));
        let (action, _) = build(
            "Finder",
            Some("com.apple.finder"),
            "open_url",
            &values(&[("url", "file:///Users/x/Downloads/x.command")]),
        )
        .unwrap()
        .unwrap();
        assert!(action.runs_code.is_some());
        assert!(
            build("X", None, "script", &values(&[("script", "get name")]))
                .unwrap()
                .is_err()
        );
        assert!(build("X", None, "open_chat", &values(&[])).is_none());
    }
}
