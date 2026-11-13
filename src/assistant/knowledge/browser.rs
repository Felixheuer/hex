//! The person's own browser: the one LaunchServices opens web links with
//! (`actions::capabilities::launch_services`), whichever it is, and its
//! open tabs, read through JavaScript for Automation (which never launches
//! it). What a browser can be asked is told by its own scripting
//! dictionary, in one of two families: Chromium's (`execute … javascript`,
//! tabs with ids) and Safari's (`do JavaScript`, tabs by place). No list
//! of browsers: a browser that speaks neither isn't scriptable this way.
//! Incognito windows are left out where the browser says which they are.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;

use super::exec;
use super::rank::{self, Hit};
use crate::assistant::actions::capabilities;

/// The system's browser when the person never chose another.
const SYSTEM_BROWSER: &str = "com.apple.Safari";

/// How a browser runs JavaScript in a tab, from its dictionary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Family {
    /// `execute tab javascript "…"`; windows and tabs have ids.
    Chromium,
    /// `do JavaScript "…" in tab`; tabs are known by their place.
    Safari,
}

impl Family {
    /// The family a scripting dictionary (`sdef` XML) speaks, if any.
    pub fn of_dictionary(xml: &str) -> Option<Self> {
        let lower = xml.to_lowercase();
        if lower.contains("\"do javascript\"") {
            Some(Self::Safari)
        } else if lower.contains("name=\"execute\"") && lower.contains("name=\"javascript\"") {
            Some(Self::Chromium)
        } else {
            None
        }
    }

    fn list_script(self) -> &'static str {
        match self {
            Self::Chromium => CHROMIUM_LIST,
            Self::Safari => SAFARI_LIST,
        }
    }

    pub(super) fn act_script(self) -> &'static str {
        match self {
            Self::Chromium => CHROMIUM_ACT,
            Self::Safari => SAFARI_ACT,
        }
    }
}

/// The person's browser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Browser {
    pub bundle_id: String,
    pub name: String,
    pub family: Family,
}

/// One open tab of the person's browser.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub window_id: i64,
    /// The window's place, front first (1 is the front window).
    pub window: usize,
    /// The browser's own id for the tab, or (where tabs have none) one
    /// made of its window's id and its place.
    pub tab_id: i64,
    /// The tab's place in its window, from 1.
    pub index: usize,
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Listing {
    running: bool,
    ids: Vec<i64>,
    modes: Vec<String>,
    active: Vec<usize>,
    tab_ids: Vec<Vec<i64>>,
    titles: Vec<Vec<String>>,
    urls: Vec<Vec<String>>,
}

const CHROMIUM_LIST: &str = r#"function run(argv) {
  var app = Application(argv[0]);
  if (!app.running()) return JSON.stringify({running: false});
  var w = app.windows;
  var modes = [];
  try { modes = w.mode(); } catch (e) {}
  return JSON.stringify({running: true, ids: w.id(), modes: modes, active: w.activeTabIndex(),
    tabIds: w.tabs.id(), titles: w.tabs.title(), urls: w.tabs.url()});
}"#;

const SAFARI_LIST: &str = r#"function run(argv) {
  var app = Application(argv[0]);
  if (!app.running()) return JSON.stringify({running: false});
  var out = {running: true, ids: [], modes: [], active: [], tabIds: [], titles: [], urls: []};
  app.windows().forEach(function (win) {
    try {
      var id = win.id();
      var names = win.tabs.name();
      var links = win.tabs.url();
      var current = win.currentTab.index();
      out.ids.push(id);
      out.modes.push('normal');
      out.active.push(current);
      out.tabIds.push(names.map(function (_, i) { return id * 1000 + i + 1; }));
      out.titles.push(names.map(function (name) { return name || ''; }));
      out.urls.push(links.map(function (url) { return url || ''; }));
    } catch (e) {}
  });
  return JSON.stringify(out);
}"#;

/// argv: bundle id, window id, tab id, tab place, operation, payload.
const CHROMIUM_ACT: &str = r#"function run(argv) {
  var app = Application(argv[0]);
  if (!app.running()) throw new Error('the browser isn\'t running');
  var win = app.windows.byId(Number(argv[1]));
  var tab = win.tabs.byId(Number(argv[2]));
  var op = argv[4];
  if (op === 'js') { var value = tab.execute({javascript: argv[5]}); return value === undefined || value === null ? '' : String(value); }
  if (op === 'navigate') { tab.url = argv[5]; return 'ok'; }
  if (op === 'show') {
    var ids = win.tabs.id();
    win.activeTabIndex = ids.indexOf(Number(argv[2])) + 1;
    win.index = 1;
    app.activate();
    return 'ok';
  }
  throw new Error('unknown operation ' + op);
}"#;

const SAFARI_ACT: &str = r#"function run(argv) {
  var app = Application(argv[0]);
  if (!app.running()) throw new Error('the browser isn\'t running');
  var win = app.windows.byId(Number(argv[1]));
  var tab = win.tabs[Number(argv[3]) - 1];
  var op = argv[4];
  if (op === 'js') { var value = app.doJavaScript(argv[5], {in: tab}); return value === undefined || value === null ? '' : String(value); }
  if (op === 'navigate') { tab.url = argv[5]; return 'ok'; }
  if (op === 'show') {
    win.currentTab = tab;
    win.index = 1;
    app.activate();
    return 'ok';
  }
  throw new Error('unknown operation ' + op);
}"#;

/// The person's browser: the app that opens `https` links, and how it is
/// scripted; why not when it can't be.
pub fn default_browser() -> Result<Browser, String> {
    let bundle_id = capabilities::launch_services()
        .and_then(|services| capabilities::handler_for(&services, "https"))
        .unwrap_or_else(|| SYSTEM_BROWSER.to_owned());
    browser(&bundle_id)
}

/// The installed browser `bundle_id`, read once per process.
fn browser(bundle_id: &str) -> Result<Browser, String> {
    static KNOWN: Mutex<Option<HashMap<String, Browser>>> = Mutex::new(None);
    let key = bundle_id.to_lowercase();
    if let Ok(known) = KNOWN.lock()
        && let Some(found) = known.as_ref().and_then(|known| known.get(&key))
    {
        return Ok(found.clone());
    }
    let path = app_path(bundle_id)
        .ok_or_else(|| format!("the person's browser ({bundle_id}) isn't installed"))?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| bundle_id.to_owned());
    let bundle_id = capabilities::info_plist(&path)
        .and_then(|info| info["CFBundleIdentifier"].as_str().map(str::to_owned))
        .unwrap_or_else(|| bundle_id.to_owned());
    let dictionary = exec::output(
        Command::new("/usr/bin/sdef").arg(&path),
        Duration::from_secs(3),
        8 * 1024 * 1024,
    )
    .unwrap_or_default();
    let family = Family::of_dictionary(&dictionary).ok_or_else(|| {
        format!(
            "the person's browser, {name}, can't run JavaScript in its tabs by script; Agent-Desk \
             reads and acts in tabs of browsers that can (Chromium-based ones and Safari)"
        )
    })?;
    let found = Browser {
        bundle_id,
        name,
        family,
    };
    if let Ok(mut known) = KNOWN.lock() {
        known
            .get_or_insert_with(HashMap::new)
            .insert(key, found.clone());
    }
    Ok(found)
}

/// Where the app `bundle_id` is installed (Spotlight).
fn app_path(bundle_id: &str) -> Option<PathBuf> {
    let id = bundle_id.replace(['\'', '"', '\\'], "");
    exec::output(
        Command::new("/usr/bin/mdfind").arg(format!("kMDItemCFBundleIdentifier == '{id}'c")),
        Duration::from_secs(3),
        64 * 1024,
    )
    .ok()?
    .lines()
    .map(PathBuf::from)
    .find(|path| path.extension().is_some_and(|extension| extension == "app"))
}

/// Runs a JavaScript-for-Automation script with `args`, for `browser`.
pub fn jxa(
    browser: &Browser,
    script: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    exec::output(
        Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", script])
            .arg(&browser.bundle_id)
            .args(args),
        timeout,
        2 * 1024 * 1024,
    )
    .map(|text| text.trim_end_matches('\n').to_owned())
    .map_err(|error| super::apple_events::explain(&error, browser))
}

/// The listing as tabs, incognito windows left out.
fn parse_listing(text: &str) -> Result<Option<Vec<Tab>>, String> {
    let listing: Listing =
        serde_json::from_str(text).map_err(|error| format!("unreadable tab list: {error}"))?;
    if !listing.running {
        return Ok(None);
    }
    let mut tabs = Vec::new();
    for (window, id) in listing.ids.iter().enumerate() {
        if listing
            .modes
            .get(window)
            .is_some_and(|mode| mode == "incognito")
        {
            continue;
        }
        let active = listing.active.get(window).copied().unwrap_or(0);
        let titles = listing.titles.get(window).cloned().unwrap_or_default();
        let urls = listing.urls.get(window).cloned().unwrap_or_default();
        let ids = listing.tab_ids.get(window).cloned().unwrap_or_default();
        for (index, tab_id) in ids.iter().enumerate() {
            tabs.push(Tab {
                window_id: *id,
                window: window + 1,
                tab_id: *tab_id,
                index: index + 1,
                title: titles.get(index).cloned().unwrap_or_default(),
                url: urls.get(index).cloned().unwrap_or_default(),
                active: index + 1 == active,
            });
        }
    }
    Ok(Some(tabs))
}

/// The open tabs of `browser`; `None` when it isn't running.
pub fn open_tabs(browser: &Browser) -> Result<Option<Vec<Tab>>, String> {
    // A browser with many tabs, or one waking up, takes seconds to list them.
    let text = jxa(
        browser,
        browser.family.list_script(),
        &[],
        Duration::from_secs(8),
    )?;
    parse_listing(&text).map_err(|error| {
        format!(
            "{} answered something that isn't its tab list ({error}): “{}”",
            browser.name,
            crate::assistant::store::shorten(text.trim(), 200)
        )
    })
}

/// `url` without a query that carries a secret (a token, key, code, or
/// session in its parameters).
pub fn safe_url(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let lower = query.to_lowercase();
    let secret = [
        "token", "key", "code", "secret", "password", "passwd", "session", "auth", "sig",
    ]
    .iter()
    .any(|word| lower.contains(word));
    if secret {
        format!("{base}?…")
    } else {
        url.to_owned()
    }
}

/// How a tab reads in a result.
pub fn tab_hit(tab: &Tab, score: f64) -> Hit {
    Hit {
        title: if tab.title.is_empty() {
            tab.url.clone()
        } else {
            tab.title.clone()
        },
        detail: format!(
            "{} · window {}, tab {}{} · tab id {}",
            safe_url(&tab.url),
            tab.window,
            tab.index,
            if tab.active { " (showing)" } else { "" },
            tab.tab_id
        ),
        when_ms: None,
        score,
    }
}

/// Tabs matching the query; the showing tab of the front window first
/// among equals. No query: every tab, front window first.
pub fn rank_tabs(tabs: &[Tab], tokens: &[String], limit: usize) -> Vec<Hit> {
    let hits = tabs
        .iter()
        .map(|tab| {
            let matched = rank::score(&format!("{} {}", tab.title, tab.url), tokens);
            let bonus = if tab.active { 0.1 } else { 0.0 } + 0.05 / tab.window as f64;
            tab_hit(tab, if matched > 0.0 { matched + bonus } else { 0.0 })
        })
        .collect();
    rank::top(hits, limit)
}

pub fn tabs(tokens: &[String], limit: usize) -> Result<Vec<Hit>, String> {
    let browser = default_browser()?;
    match open_tabs(&browser)? {
        None => Err(format!("{} isn't running", browser.name)),
        Some(tabs) => Ok(rank_tabs(&tabs, tokens, limit)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_family_comes_from_the_dictionary() {
        let chromium = r#"<command name="execute" code="CrSuExJa"><parameter name="javascript" code="JvSc"/></command>"#;
        let safari = r#"<command name="do JavaScript" code="sfridojs"/>"#;
        assert_eq!(Family::of_dictionary(chromium), Some(Family::Chromium));
        assert_eq!(Family::of_dictionary(safari), Some(Family::Safari));
        assert_eq!(
            Family::of_dictionary(r#"<command name="open location"/>"#),
            None
        );
        assert!(Family::Chromium.act_script().contains("tab.execute"));
        assert!(Family::Safari.act_script().contains("doJavaScript"));
        assert!(Family::Safari.list_script().contains("currentTab"));
    }

    #[test]
    fn the_tab_listing_parses_without_incognito_windows() {
        let text = json!({"running": true, "ids": [11, 12, 13], "modes": ["normal", "incognito", "normal"],
            "active": [2, 1, 1],
            "tabIds": [[101, 102], [201], [301]],
            "titles": [["Gmail", "Kalibermarkt – Staging"], ["Secret"], ["GitHub"]],
            "urls": [["https://mail.google.com/", "https://staging.kalibermarkt.de/"], ["https://x"], ["https://github.com/"]]})
        .to_string();
        let tabs = parse_listing(&text).unwrap().unwrap();
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[1].tab_id, 102);
        assert!(tabs[1].active && !tabs[0].active);
        assert_eq!(tabs[2].window, 3);
        assert!(tabs.iter().all(|tab| tab.title != "Secret"));
        assert_eq!(parse_listing(r#"{"running":false}"#).unwrap(), None);
        // A browser that names no modes (Safari) keeps every window.
        let plain = json!({"running": true, "ids": [7], "active": [1],
            "tabIds": [[7001]], "titles": [["A"]], "urls": [["https://a/"]]})
        .to_string();
        assert_eq!(parse_listing(&plain).unwrap().unwrap().len(), 1);

        let hits = rank_tabs(&tabs, &rank::tokens("kalibermarkt"), 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].detail,
            "https://staging.kalibermarkt.de/ · window 1, tab 2 (showing) · tab id 102"
        );
        assert_eq!(rank_tabs(&tabs, &[], 5)[0].title, "Kalibermarkt – Staging");
        assert_eq!(
            safe_url("https://x.de/reset?token=abc"),
            "https://x.de/reset?…"
        );
    }
}
