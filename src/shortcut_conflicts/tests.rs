use serde_json::{Value, json};

use super::apps::{Apps, Running};
use super::system::{Globe, System};
use super::*;
use crate::app_settings::{HotkeyKey, HotkeyModifiers};

/// `com.apple.symbolichotkeys` as macOS saves it: Spotlight (64) turned off
/// and moved, Finder search (65) left out (on with ⌥⌘ Space), the previous
/// input source (60) on ⌃ Space, and Mission Control (32) off.
const SYMBOLIC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AppleSymbolicHotKeys</key>
  <dict>
    <key>64</key>
    <dict>
      <key>enabled</key><false/>
      <key>value</key>
      <dict>
        <key>parameters</key>
        <array><integer>32</integer><integer>49</integer><integer>1048576</integer></array>
        <key>type</key><string>standard</string>
      </dict>
    </dict>
    <key>60</key>
    <dict>
      <key>enabled</key><true/>
      <key>value</key>
      <dict>
        <key>parameters</key>
        <array><integer>32</integer><integer>49</integer><integer>262144</integer></array>
        <key>type</key><string>standard</string>
      </dict>
    </dict>
    <key>32</key>
    <dict><key>enabled</key><false/></dict>
    <key>164</key>
    <dict>
      <key>enabled</key><true/>
      <key>value</key>
      <dict>
        <key>parameters</key>
        <array><integer>65535</integer><integer>65535</integer><integer>0</integer></array>
        <key>type</key><string>standard</string>
      </dict>
    </dict>
    <key>250</key>
    <dict>
      <key>enabled</key><true/>
      <key>value</key>
      <dict>
        <key>parameters</key>
        <array><integer>100</integer><integer>2</integer><integer>1572864</integer></array>
        <key>type</key><string>standard</string>
      </dict>
    </dict>
  </dict>
</dict>
</plist>"#;

/// `com.apple.HIToolbox` with 🌐 starting Dictation and Dictation on.
const TOOLBOX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AppleFnUsageType</key><integer>3</integer>
  <key>AppleDictationAutoEnable</key><integer>1</integer>
  <key>AppleCurrentKeyboardLayoutInputSourceID</key><string>com.apple.keylayout.US</string>
</dict>
</plist>"#;

/// `com.apple.assistant.support` with Siri on.
const ASSISTANT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Assistant Enabled</key><true/>
  <key>Dictation Enabled</key><false/>
</dict>
</plist>"#;

fn system() -> System {
    System::from_domains(
        &plist::parse(SYMBOLIC),
        &plist::parse(TOOLBOX),
        &plist::parse(ASSISTANT),
        &Value::Null,
    )
}

fn chord(modifiers: HotkeyModifiers, code: u16, label: &str) -> HotkeyBinding {
    HotkeyBinding {
        modifiers,
        key: Some(HotkeyKey {
            code,
            label: label.into(),
        }),
    }
}

fn space(modifiers: HotkeyModifiers) -> HotkeyBinding {
    chord(modifiers, 49, "Space")
}

fn command() -> HotkeyModifiers {
    HotkeyModifiers {
        command: Some(ModifierSide::Either),
        ..HotkeyModifiers::default()
    }
}

fn environment(system: System, apps: Apps) -> Environment {
    Environment { system, apps }
}

fn names(found: &[Conflict]) -> Vec<(&str, Certainty)> {
    found
        .iter()
        .map(|conflict| (conflict.name.as_str(), conflict.certainty))
        .collect()
}

#[test]
fn a_fixture_plist_reads_every_saved_symbolic_hot_key() {
    let saved = system::symbolic_hotkeys(&plist::parse(SYMBOLIC));
    assert_eq!(
        saved.get(&64),
        Some(&Entry {
            enabled: false,
            chord: Some((49, 0x10_0000))
        })
    );
    assert_eq!(
        saved.get(&32),
        Some(&Entry {
            enabled: false,
            chord: None
        })
    );
    // An entry that names no key has no chord.
    assert_eq!(saved.get(&164).and_then(|entry| entry.chord), None);
    assert!(!saved.contains_key(&65));
}

/// An enabled macOS shortcut on the same keys takes the shortcut; one turned
/// off frees it; one left unsaved is on with its default.
#[test]
fn macos_keyboard_shortcuts_take_their_chords() {
    let env = environment(system(), Apps::default());
    let control = HotkeyModifiers::control();
    assert_eq!(
        names(&check(&space(control), false, &env)),
        [("Select the previous input source", Certainty::Taken)]
    );
    // Spotlight is off here, so ⌘ Space is free of macOS shortcuts (Siri
    // may still hold it).
    let found = check(&space(command()), false, &env);
    assert!(
        found
            .iter()
            .all(|conflict| conflict.name != "Show Spotlight search")
    );
    // Finder search is unsaved: on with ⌥⌘ Space.
    assert_eq!(
        names(&check(
            &space(HotkeyModifiers::option_command()),
            false,
            &env
        )),
        [("Show Finder search window", Certainty::Taken)]
    );
    // An unknown ID that is on still takes its chord (⌥⌘ D).
    let found = check(
        &chord(HotkeyModifiers::option_command(), 2, "D"),
        false,
        &env,
    );
    assert_eq!(
        names(&found),
        [("a macOS keyboard shortcut", Certainty::Taken)]
    );
    assert_eq!(found[0].fix, Fix::KeyboardSettings);
    // A side-specific chord is taken all the same: macOS matches either side.
    let right = HotkeyModifiers {
        control: Some(ModifierSide::Right),
        ..HotkeyModifiers::default()
    };
    assert_eq!(check(&space(right), false, &env).len(), 1);
    // Something nobody uses is free.
    assert!(
        check(
            &chord(HotkeyModifiers::control_option(), 98, "F7"),
            false,
            &env
        )
        .is_empty()
    );
}

/// 🌐 alone collides with its own action; Apple Dictation's and Siri's
/// press-twice shortcuts matter only to a shortcut that is double-tapped.
#[test]
fn the_globe_key_dictation_and_siri_flag_lone_modifiers() {
    let env = environment(system(), Apps::default());
    let globe = HotkeyBinding {
        modifiers: HotkeyModifiers {
            function: true,
            ..HotkeyModifiers::default()
        },
        key: None,
    };
    assert_eq!(
        names(&check(&globe, false, &env)),
        [("🌐 key", Certainty::Taken)]
    );
    assert!(
        check(&globe, false, &env)[0]
            .description
            .contains("starts Apple Dictation")
    );
    assert_eq!(
        names(&check(&globe, true, &env)),
        [
            ("🌐 key", Certainty::Taken),
            ("Apple Dictation", Certainty::Possible)
        ]
    );

    let right_command = HotkeyBinding::action_default();
    assert!(check(&right_command, false, &env).is_empty());
    assert_eq!(
        names(&check(&right_command, true, &env)),
        [
            ("Apple Dictation", Certainty::Possible),
            ("Siri", Certainty::Possible)
        ]
    );
    // Siri's hold ⌘ Space.
    let found = check(&space(command()), false, &env);
    assert_eq!(names(&found), [("Siri", Certainty::Possible)]);
    assert_eq!(found[0].fix, Fix::SiriSettings);

    // Siri off, Dictation off, 🌐 set to do nothing: nothing to flag.
    let quiet = System::from_domains(
        &Value::Null,
        &json!({ "AppleFnUsageType": 0 }),
        &json!({ "Assistant Enabled": 0 }),
        &Value::Null,
    );
    let env = environment(quiet, Apps::default());
    assert!(check(&globe, true, &env).is_empty());
    assert!(check(&right_command, true, &env).is_empty());
    // Option alone is nobody's here.
    let option = HotkeyBinding {
        modifiers: HotkeyModifiers::option(),
        key: None,
    };
    assert!(check(&option, true, &environment(system(), Apps::default())).is_empty());
    assert_eq!(globe_of(&json!({})), Globe::Default);
}

fn globe_of(toolbox: &Value) -> Globe {
    System::from_domains(&Value::Null, toolbox, &Value::Null, &Value::Null).globe
}

/// macOS with its Space shortcuts (Spotlight, Finder search, input
/// sources) turned off, so only the apps take keys.
fn no_space_shortcuts() -> System {
    let off = json!({ "enabled": 0 });
    let symbolic = json!({ "AppleSymbolicHotKeys": {
        "60": off, "61": off, "64": off, "65": off
    } });
    System::from_domains(&symbolic, &Value::Null, &Value::Null, &Value::Null)
}

fn running(name: &str, bundle_id: &str) -> Running {
    Running {
        name: name.into(),
        bundle_id: bundle_id.into(),
    }
}

/// A running dictation app may take any shortcut; a launcher only a chord
/// on Space, exactly when its hot key is readable.
#[test]
fn running_apps_flag_the_keys_they_commonly_take() {
    let apps = Apps {
        running: vec![
            running("Wispr Flow", "com.electron.wispr-flow"),
            running("Alfred", "com.runningwithcrayons.Alfred"),
        ],
        ..Apps::default()
    };
    let env = environment(no_space_shortcuts(), apps);
    let right_command = HotkeyBinding::action_default();
    let found = check(&right_command, false, &env);
    assert_eq!(names(&found), [("Wispr Flow", Certainty::Possible)]);
    assert_eq!(
        found[0].fix,
        Fix::OpenApp {
            bundle_id: "com.electron.wispr-flow".into()
        }
    );
    assert_eq!(
        names(&check(&space(HotkeyModifiers::option()), false, &env)),
        [
            ("Wispr Flow", Certainty::Possible),
            ("Alfred", Certainty::Possible)
        ]
    );

    // Raycast's saved hot key decides exactly.
    let raycast = |saved: &str| Apps {
        running: vec![running("Raycast", "com.raycast.macos")],
        raycast_hotkey: Some(saved.into()),
        ..Apps::default()
    };
    let env = environment(no_space_shortcuts(), raycast("Option-49"));
    assert_eq!(
        names(&check(&space(HotkeyModifiers::option()), false, &env)),
        [("Raycast", Certainty::Taken)]
    );
    assert!(check(&space(command()), false, &env).is_empty());
    assert_eq!(
        apps::raycast_hotkey("Command-Shift-49"),
        Some((Some(49), 0b1001))
    );
    assert_eq!(apps::raycast_hotkey("Hyper-49"), None);
    // Unreadable: a possible conflict on Space.
    let env = environment(no_space_shortcuts(), raycast("⌘ Space"));
    assert_eq!(
        names(&check(&space(command()), false, &env)),
        [("Raycast", Certainty::Possible)]
    );
}

/// Karabiner-Elements' selected profile: a simple modification of one of
/// the shortcut's keys takes it; a complex rule from it may.
#[test]
fn karabiner_rules_on_the_shortcuts_keys_are_flagged() {
    let config = json!({
        "profiles": [
            { "name": "Other", "simple_modifications": [
                { "from": { "key_code": "left_option" }, "to": [{ "key_code": "f18" }] }
            ] },
            { "name": "Default", "selected": true,
              "simple_modifications": [
                { "from": { "key_code": "caps_lock" }, "to": [{ "key_code": "escape" }] }
              ],
              "devices": [ { "simple_modifications": [
                { "from": { "key_code": "right_command" }, "to": [{ "key_code": "f19" }] }
              ] } ],
              "complex_modifications": { "rules": [ { "manipulators": [
                { "from": { "key_code": "spacebar", "modifiers": { "mandatory": ["option"] } },
                  "to": [{ "shell_command": "open -a Notes" }] }
              ] } ] } }
        ]
    });
    let env = environment(
        System::default(),
        Apps {
            karabiner: Some(config),
            ..Apps::default()
        },
    );
    assert_eq!(
        names(&check(&HotkeyBinding::action_default(), true, &env)),
        [("Karabiner-Elements", Certainty::Taken)]
    );
    assert_eq!(
        names(&check(&space(HotkeyModifiers::option()), false, &env)),
        [("Karabiner-Elements", Certainty::Possible)]
    );
    // Left Option is only remapped in a profile that isn't selected.
    let left_option = HotkeyBinding {
        modifiers: HotkeyModifiers {
            option: Some(ModifierSide::Left),
            ..HotkeyModifiers::default()
        },
        key: None,
    };
    assert!(check(&left_option, false, &env).is_empty());
}

/// The slots read their own saved shortcut, and only the Action shortcut is
/// always double-tapped.
#[test]
fn each_slot_checks_its_own_shortcut() {
    let mut settings = AppSettings {
        double_tap_lock: false,
        ..AppSettings::default()
    };
    assert_eq!(
        shortcut(HotkeySlot::Action, &settings),
        Some((HotkeyBinding::action_default(), true))
    );
    assert_eq!(
        shortcut(HotkeySlot::Dictation, &settings),
        Some((settings.dictation_hotkey.clone(), false))
    );
    settings.assistant.action_hotkey = None;
    assert_eq!(shortcut(HotkeySlot::Action, &settings), None);
    assert!(conflicts(HotkeySlot::Action, &settings).is_empty());
    // Stop all tasks is a press, never double-tapped, even with the lock on.
    settings.double_tap_lock = true;
    assert_eq!(
        shortcut(HotkeySlot::StopAll, &settings),
        Some((HotkeyBinding::stop_all_default(), false))
    );
    settings.assistant.stop_all_hotkey = None;
    assert_eq!(shortcut(HotkeySlot::StopAll, &settings), None);
    assert!(conflicts(HotkeySlot::StopAll, &settings).is_empty());
}
