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

mod bundle;
mod sdef;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use bundle::plist_json;
pub use bundle::{
    Bundle, Ui, bundle_of, default_schemes, handler_for, info_plist, launch_services, ui_of,
};
pub use sdef::{search_example, summarize_sdef};

/// Most characters of a dictionary summary.
pub const DICTIONARY_BUDGET: usize = 4000;

/// Most document type names listed.
const MAX_DOCUMENT_TYPES: usize = 8;

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
        assert_eq!(
            handler_for(&services, "HTTPS").as_deref(),
            Some("org.example.browser")
        );
        assert_eq!(handler_for(&services, "ftp"), None);
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
