//! `hex_chrome_tab`: the tab the person means ("my open tab") in their own
//! Google Chrome, read and acted in where it is: no hidden copy, no mouse,
//! no keyboard. JavaScript runs in the tab through Apple Events (Chrome's
//! View › Developer › Allow JavaScript from Apple Events); without it a
//! read fetches the URL fresh instead.
//!
//! A click is a consequential action like any other desktop action: its
//! target's label is classified (`autonomy::classify`) and the autonomy
//! mode decides (`autonomy::allowed`), so in Ask and Smart a "Send" or
//! "Buy now" the person didn't ask for comes back as `NEEDS_CONFIRMATION`
//! and Yolo runs it. Scripts may only read unless the mode is Yolo.

use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

use super::chrome::{self, Tab};
use super::exec;
use super::rank;
use crate::assistant::autonomy::{self, Authorized, Autonomy};

/// How much of a page a read returns by default, and at most.
const READ_CHARS: usize = 8000;
const READ_MAX: usize = 40_000;
const TIMEOUT: Duration = Duration::from_secs(8);

pub fn definition() -> Value {
    json!({
        "name": "hex_chrome_tab",
        "description": "The person's OWN Google Chrome (what they see, signed in): read or act in a tab that is already open, without a hidden copy, the mouse, or the keyboard. Use it when the person means a tab they have open (\u{201c}my open tab\u{201d}, \u{201c}the page I'm on\u{201d}, \u{201c}in Chrome\u{201d}); use the agent browser for anything else. `tab` picks the tab: its id (from hex_lookup scope chrome_tabs or an earlier call), or words of its title or URL; empty means the tab showing in the front Chrome window. Actions: read (title, URL, page text), click (a CSS `selector` or the visible `text` of a button or link), fill (a field by `selector` or its label/placeholder `text`, with `value`), navigate (`url`, in that same tab), run (a read-only JavaScript expression `script`; its value comes back), show (bring the tab to the front, only when the person wants to see it). Clicks on send, delete, buy, share, or publish follow the person's autonomy setting.",
        "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["read", "click", "fill", "navigate", "run", "show"], "description": "Default read."},
            "tab": {"type": "string", "description": "Tab id, or words of its title or URL; empty: the showing tab of the front window."},
            "selector": {"type": "string", "description": "CSS selector of the element (click, fill)."},
            "text": {"type": "string", "description": "Visible text of the button or link (click), or the field's label or placeholder (fill)."},
            "value": {"type": "string", "description": "What to type into the field (fill)."},
            "url": {"type": "string", "description": "http(s) URL to open in the tab (navigate)."},
            "script": {"type": "string", "description": "A JavaScript expression evaluated in the page (run)."},
            "max_chars": {"type": "integer", "description": "How much page text a read returns (default 8000, at most 40000)."}
        }},
    })
}

/// Which tab `wanted` names among `tabs`: an id, else the best title/URL
/// match, else (empty) the front window's showing tab.
pub fn choose<'a>(tabs: &'a [Tab], wanted: &str) -> Result<&'a Tab, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() || matches!(wanted, "active" | "current" | "front") {
        return tabs
            .iter()
            .find(|tab| tab.active)
            .or(tabs.first())
            .ok_or_else(|| "Chrome has no open tab".into());
    }
    let id = wanted
        .trim_start_matches("tab")
        .trim_start_matches(" id")
        .trim()
        .parse::<i64>();
    if let Ok(id) = id
        && let Some(tab) = tabs.iter().find(|tab| tab.tab_id == id)
    {
        return Ok(tab);
    }
    let tokens = rank::tokens(wanted);
    tabs.iter()
        .map(|tab| {
            let score = rank::score(&format!("{} {}", tab.title, tab.url), &tokens)
                + if tab.active { 0.05 } else { 0.0 };
            (score, tab)
        })
        .filter(|(score, _)| *score > 0.05)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, tab)| tab)
        .ok_or_else(|| format!("No open Chrome tab matches “{wanted}”; list them with hex_lookup scope chrome_tabs."))
}

const ACT_SCRIPT: &str = r#"function run(argv) {
  var chrome = Application('Google Chrome');
  if (!chrome.running()) throw new Error('Google Chrome isn\'t running');
  var win = chrome.windows.byId(Number(argv[0]));
  var tab = win.tabs.byId(Number(argv[1]));
  var op = argv[2];
  if (op === 'js') { var value = tab.execute({javascript: argv[3]}); return value === undefined || value === null ? '' : String(value); }
  if (op === 'navigate') { tab.url = argv[3]; return 'ok'; }
  if (op === 'show') {
    var ids = win.tabs.id();
    win.activeTabIndex = ids.indexOf(Number(argv[1])) + 1;
    win.index = 1;
    chrome.activate();
    return 'ok';
  }
  throw new Error('unknown operation ' + op);
}"#;

fn act(tab: &Tab, op: &str, payload: &str) -> Result<String, String> {
    chrome::jxa(
        ACT_SCRIPT,
        &[
            &tab.window_id.to_string(),
            &tab.tab_id.to_string(),
            op,
            payload,
        ],
        TIMEOUT,
    )
}

/// Whether Chrome refused JavaScript from Apple Events.
fn javascript_off(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("javascript") && (lower.contains("turned off") || lower.contains("allow"))
}

const JAVASCRIPT_OFF: &str = "Chrome doesn't allow JavaScript from Apple Events, so Hex can't act in the tab. The person can turn it on in Chrome: View › Developer › Allow JavaScript from Apple Events.";

/// A JS string literal.
fn js(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

pub fn read_script(max_chars: usize) -> String {
    format!(
        "(function(){{var b=document.body;var t=b?b.innerText:'';return JSON.stringify({{title:document.title,url:location.href,length:t.length,text:t.slice(0,{max_chars}),selection:String(getSelection()).slice(0,2000)}});}})()"
    )
}

/// Finds the element (by `selector`, else by its visible text or label),
/// marks it with `nonce`, and tells what it is.
pub fn find_script(selector: &str, text: &str, fields: bool, nonce: &str) -> String {
    let candidates = if fields {
        "input:not([type=hidden]),textarea,select,[contenteditable=''],[contenteditable=true],[role=textbox],[role=combobox]"
    } else {
        "a,button,input[type=submit],input[type=button],input[type=checkbox],input[type=radio],[role=button],[role=link],[role=menuitem],[role=tab],[role=checkbox],[role=option],summary,label"
    };
    format!(
        "(function(sel,text,nonce){{\
function label(e){{var l=e.labels&&e.labels[0]?e.labels[0].innerText:'';return [e.innerText||'',e.getAttribute('aria-label')||'',e.getAttribute('placeholder')||'',e.getAttribute('name')||'',e.getAttribute('title')||'',(e.type==='submit'||e.type==='button')?e.value:'',l].join(' ').replace(/\\s+/g,' ').trim().slice(0,200);}}\
var el=null;\
if(sel){{el=document.querySelector(sel);}}else{{var want=text.trim().toLowerCase().replace(/\\s+/g,' ');var all=Array.prototype.slice.call(document.querySelectorAll({candidates}));\
el=all.find(function(e){{return label(e).toLowerCase()===want;}})||all.find(function(e){{return label(e).toLowerCase().indexOf(want)>=0;}});}}\
if(!el)return JSON.stringify({{found:false}});\
el.setAttribute('data-hex-target',nonce);\
return JSON.stringify({{found:true,label:label(el),tag:el.tagName.toLowerCase(),type:(el.type||'').toLowerCase()}});\
}})({},{},{})",
        js(selector),
        js(text),
        js(nonce),
        candidates = js(candidates),
    )
}

pub fn click_script(nonce: &str) -> String {
    format!(
        "(function(n){{var e=document.querySelector('[data-hex-target=\"'+n+'\"]');if(!e)return 'gone';e.removeAttribute('data-hex-target');e.scrollIntoView({{block:'center'}});e.click();return 'clicked';}})({})",
        js(nonce)
    )
}

pub fn fill_script(nonce: &str, value: &str) -> String {
    format!(
        "(function(n,v){{var e=document.querySelector('[data-hex-target=\"'+n+'\"]');if(!e)return 'gone';e.removeAttribute('data-hex-target');e.focus();\
if(e.isContentEditable){{document.execCommand('selectAll',false,null);document.execCommand('insertText',false,v);return 'filled';}}\
var proto=e instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:e instanceof HTMLSelectElement?HTMLSelectElement.prototype:HTMLInputElement.prototype;\
Object.getOwnPropertyDescriptor(proto,'value').set.call(e,v);\
e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return 'filled';}})({},{})",
        js(nonce),
        js(value)
    )
}

/// Page HTML as plain text: scripts and styles dropped, tags removed,
/// common entities decoded, whitespace collapsed.
pub fn html_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len() / 3);
    let lower = html.to_ascii_lowercase();
    let mut index = 0;
    while index < html.len() {
        let rest = &lower[index..];
        if rest.starts_with("<script")
            || rest.starts_with("<style")
            || rest.starts_with("<noscript")
        {
            let close = if rest.starts_with("<script") {
                "</script>"
            } else if rest.starts_with("<style") {
                "</style>"
            } else {
                "</noscript>"
            };
            index += rest.find(close).map_or(rest.len(), |at| at + close.len());
            continue;
        }
        if rest.starts_with('<') {
            index += rest.find('>').map_or(rest.len(), |at| at + 1);
            text.push(' ');
            continue;
        }
        let next = rest.find('<').unwrap_or(rest.len());
        text.push_str(&html[index..index + next]);
        index += next;
    }
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fetch_text(url: &str, max_chars: usize) -> Result<String, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("the tab shows no web page".into());
    }
    let html = exec::output(
        Command::new("/usr/bin/curl")
            .args(["-sL", "--max-time", "6", "--max-filesize", "5000000", "-A"])
            .arg("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
            .arg(url),
        Duration::from_secs(7),
        5_000_000,
    )?;
    Ok(html_text(&html).chars().take(max_chars).collect())
}

fn header(tab: &Tab) -> String {
    format!(
        "Tab id {} (window {}, tab {}{}): {}\n{}\n",
        tab.tab_id,
        tab.window,
        tab.index,
        if tab.active { ", showing" } else { "" },
        tab.title,
        chrome::safe_url(&tab.url)
    )
}

fn read(tab: &Tab, max_chars: usize) -> Result<String, String> {
    match act(tab, "js", &read_script(max_chars)) {
        Ok(json) => {
            let page: Value = serde_json::from_str(&json).unwrap_or_default();
            let text = page["text"].as_str().unwrap_or_default();
            let length = page["length"].as_u64().unwrap_or(0) as usize;
            let mut out = header(tab);
            if let Some(selection) = page["selection"]
                .as_str()
                .filter(|text| !text.trim().is_empty())
            {
                out.push_str(&format!("Selected: {selection}\n"));
            }
            out.push('\n');
            out.push_str(text);
            if length > text.chars().count() {
                out.push_str(&format!(
                    "\n… ({length} characters in all; pass max_chars for more)"
                ));
            }
            Ok(out)
        }
        Err(error) if javascript_off(&error) => {
            let text = fetch_text(&tab.url, max_chars)?;
            Ok(format!(
                "{}\n(Chrome doesn't allow JavaScript from Apple Events, so this is the page fetched fresh, without the person's sign-in; turn it on in Chrome: View › Developer › Allow JavaScript from Apple Events.)\n\n{text}",
                header(tab)
            ))
        }
        Err(error) => Err(error),
    }
}

/// The person's autonomy mode and what this task may do.
fn policy(task: Option<&str>) -> (Autonomy, Authorized) {
    match task {
        Some(task) => (crate::assistant::browser::policy(task))(),
        None => (
            crate::app_settings::AppSettings::read()
                .map(|settings| settings.assistant.autonomy)
                .unwrap_or_default(),
            Authorized::default(),
        ),
    }
}

/// Whether clicking `label` may run now; the refusal to answer with if not.
pub fn check_click(label: &str, mode: Autonomy, authorized: &Authorized) -> Result<(), String> {
    let Some(category) = autonomy::classify(label) else {
        return Ok(());
    };
    if autonomy::allowed(mode, category, authorized) {
        return Ok(());
    }
    let refusal = crate::assistant::browser::consent::needs_confirmation(
        "hex_chrome_tab",
        mode,
        category,
        &format!("click “{label}” in the person's Chrome tab"),
    );
    Err(refusal["content"][0]["text"]
        .as_str()
        .unwrap_or("NEEDS_CONFIRMATION")
        .to_owned())
}

fn nonce() -> String {
    format!(
        "hex{:x}",
        crate::assistant::store::now_ms() ^ u64::from(std::process::id())
    )
}

/// Finds the element for a click or fill; its label and type.
fn find(
    tab: &Tab,
    arguments: &Value,
    fields: bool,
    nonce: &str,
) -> Result<(String, String), String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    if text("selector").is_empty() && text("text").is_empty() {
        return Err("hex_chrome_tab needs “selector” or “text” to find the element".into());
    }
    let found = act(
        tab,
        "js",
        &find_script(text("selector"), text("text"), fields, nonce),
    )
    .map_err(|error| {
        if javascript_off(&error) {
            JAVASCRIPT_OFF.into()
        } else {
            error
        }
    })?;
    let found: Value = serde_json::from_str(&found).unwrap_or_default();
    if found["found"] != json!(true) {
        return Err("No such element in the tab; read it first and pick a selector or the exact visible text.".into());
    }
    Ok((
        found["label"].as_str().unwrap_or_default().to_owned(),
        found["type"].as_str().unwrap_or_default().to_owned(),
    ))
}

/// Runs the tool for the task `task` (whose authorization applies).
pub fn call(arguments: &Value, task: Option<&str>) -> Result<String, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let tabs = chrome::open_tabs()?.ok_or("Google Chrome isn't running")?;
    let tab = choose(&tabs, text("tab"))?;
    match text("action") {
        "" | "read" => {
            let max = arguments
                .get("max_chars")
                .and_then(Value::as_u64)
                .map_or(READ_CHARS, |max| (max as usize).clamp(200, READ_MAX));
            read(tab, max)
        }
        "click" => {
            let nonce = nonce();
            let (label, _) = find(tab, arguments, false, &nonce)?;
            let (mode, authorized) = policy(task);
            check_click(&label, mode, &authorized)?;
            let done = act(tab, "js", &click_script(&nonce))?;
            if done == "gone" {
                return Err("The element went away before the click; read the tab again.".into());
            }
            Ok(format!(
                "Clicked “{label}” in tab id {}. Read the tab to see what changed.",
                tab.tab_id
            ))
        }
        "fill" => {
            let nonce = nonce();
            let (label, kind) = find(tab, arguments, true, &nonce)?;
            if kind == "password" {
                return Err(
                    "Hex never types into password fields; the person signs in themselves.".into(),
                );
            }
            let done = act(tab, "js", &fill_script(&nonce, text("value")))?;
            if done == "gone" {
                return Err("The field went away; read the tab again.".into());
            }
            Ok(format!("Filled “{label}” in tab id {}.", tab.tab_id))
        }
        "navigate" => {
            let url = text("url").trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("navigate needs an http(s) “url”".into());
            }
            act(tab, "navigate", url)?;
            Ok(format!("Tab id {} now opens {url}.", tab.tab_id))
        }
        "run" => {
            let script = text("script");
            if script.trim().is_empty() {
                return Err("run needs a “script”".into());
            }
            let (mode, _) = policy(task);
            if mode != Autonomy::Yolo && crate::assistant::browser::consent::acts(script) {
                return Err("Scripts in the person's tab may read but not click, submit, or send; use action click or fill.".into());
            }
            let value = act(tab, "js", script).map_err(|error| {
                if javascript_off(&error) {
                    JAVASCRIPT_OFF.into()
                } else {
                    error
                }
            })?;
            Ok(crate::assistant::store::shorten(&value, READ_MAX))
        }
        "show" => {
            act(tab, "show", "")?;
            Ok(format!("Tab id {} is in front now.", tab.tab_id))
        }
        other => Err(format!("hex_chrome_tab has no action “{other}”")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs() -> Vec<Tab> {
        let tab = |tab_id, window, active, title: &str, url: &str| Tab {
            window_id: window as i64 * 10,
            window,
            tab_id,
            index: 1,
            title: title.into(),
            url: url.into(),
            active,
        };
        vec![
            tab(
                1,
                1,
                false,
                "Posteingang – Gmail",
                "https://mail.google.com/mail/u/0/",
            ),
            tab(
                2,
                1,
                true,
                "Kalibermarkt Staging",
                "https://staging.kalibermarkt.de/",
            ),
            tab(
                3,
                2,
                true,
                "GitHub – Felixheuer/hex",
                "https://github.com/Felixheuer/hex",
            ),
        ]
    }

    #[test]
    fn the_tab_is_found_by_id_words_or_what_shows() {
        let tabs = tabs();
        assert_eq!(choose(&tabs, "").unwrap().tab_id, 2);
        assert_eq!(choose(&tabs, "3").unwrap().tab_id, 3);
        assert_eq!(choose(&tabs, "tab id 1").unwrap().tab_id, 1);
        assert_eq!(choose(&tabs, "gmail").unwrap().tab_id, 1);
        assert_eq!(choose(&tabs, "github hex").unwrap().tab_id, 3);
        assert!(choose(&tabs, "99").unwrap_err().contains("No open"));
        assert!(choose(&tabs, "ebay").is_err());
        assert!(choose(&[], "").is_err());
    }

    #[test]
    fn consequential_clicks_follow_the_autonomy_mode() {
        let none = Authorized::default();
        assert!(check_click("Senden", Autonomy::Yolo, &none).is_ok());
        assert!(check_click("Weiter", Autonomy::Ask, &none).is_ok());
        let refused = check_click("Senden", Autonomy::Smart, &none).unwrap_err();
        assert!(refused.contains("NEEDS_CONFIRMATION"), "{refused}");
        let mut sending = Authorized::default();
        sending.categories.insert(autonomy::Category::Send);
        assert!(check_click("Send", Autonomy::Smart, &sending).is_ok());
        assert!(check_click("Jetzt kaufen", Autonomy::Smart, &sending).is_err());
    }

    #[test]
    fn scripts_carry_their_inputs_as_literals() {
        let find = find_script("", "it's \"Send\"", false, "hex1");
        assert!(find.contains(r#""it's \"Send\"""#), "{find}");
        assert!(find.ends_with(r#"})("","it's \"Send\"","hex1")"#), "{find}");
        let fill = fill_script("hex1", "a'b\n</script>");
        assert!(fill.ends_with(r#"})("hex1","a'b\n</script>")"#), "{fill}");
        assert!(read_script(100).contains("t.slice(0,100)"));
        assert!(click_script("hex1").contains("e.click()"));
    }

    #[test]
    fn javascript_off_is_recognized_and_pages_read_as_text() {
        assert!(javascript_off(
            "execution error: Google Chrome got an error: Executing JavaScript through AppleScript is turned off. (12)"
        ));
        assert!(!javascript_off("Not authorized to send Apple events"));
        assert_eq!(
            html_text(
                "<html><head><style>p{}</style><script>var a='<b>';</script></head><body><h1>Hi&nbsp;there</h1><p>A &amp; B</p></body></html>"
            ),
            "Hi there A & B"
        );
    }
}
