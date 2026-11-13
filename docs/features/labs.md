# Labs (retired)

Labs is gone. Pilot is an assistant that operates the Mac, with dictation as a
side feature, and a store of 75 experimental switches made the window harder to
understand than the product. There is no Labs pane, no Labs sidebar item, no
`hex labs` command, no `hex://labs` page and no `labs.*` settings anymore.

Every former lab was decided once:

- **Graduated**: always on where the OS has it, with no switch.
- **A setting**: an ordinary choice in `settings.json` › `preferences`, shown
  in the window where it belongs.
- **Removed**: off for good; the window offers nothing.

The runtime gates live in `src/features.rs` (`crate::features`), the settings in
`src/app_settings/preferences.rs` (`AppSettings::preferences`).

## Decisions

| Former lab | Decision | Where it is now |
| --- | --- | --- |
| Live text | Setting, on by default | Dictation › Text › Live text (`preferences.live_text`) |
| Live typing (Linux) | Setting, off | Dictation › Text › Live typing (`preferences.live_typing`) |
| Keep dictation on clipboard, Copy only | One setting | Dictation › Text › After dictating: Paste / Paste and keep on clipboard / Copy only (`preferences.after_dictation`) |
| Spoken formatting, Spoken symbols | One setting, on by default | Dictation › Text › Spoken formatting (`preferences.spoken_formatting`) |
| Filler words | Setting, on by default | Dictation › Text › Remove filler words (`preferences.remove_filler_words`) |
| Unload model when idle (+ minutes) | One dropdown | Dictation › Advanced › Unload the speech model: Never / 5–60 min (`preferences.unload_model_after_minutes`) |
| Keep mic open after dictating (+ seconds) | One dropdown | Dictation › Advanced › Keep the mic open after dictating, while the mic turns off between dictations (`preferences.keep_microphone_open_seconds`) |
| Caps Lock dictation (+ light) | One dropdown (macOS) | Dictation › Advanced › Caps Lock: Off / light steady / light blinks (`preferences.caps_lock`) |
| Appearance (+ theme) | One dropdown | General › Appearance: System / Light / Dark (`preferences.appearance`) |
| Hide menu bar icon | Setting (macOS) | General (`preferences.hide_menu_bar_icon`) |
| Quit closes window | Setting, on by default (macOS) | General › ⌘Q closes the window (`preferences.quit_closes_window`) |
| Only on speakers | Setting | Dictation › Advanced › Output, under While dictating (`preferences.speakers_only`) |
| Keep recordings | Setting | History's menu › Keep audio too (`preferences.keep_recordings`) |
| Meetings | Setting (existing `meetings.enabled`) | Meetings pane's opt-in card; the pane is always in the sidebar on macOS |
| Pause the browser | Setting (existing `pause_browser_media`, macOS) | Dictation › Advanced › Output, under While dictating = Pause media |
| Switch model with keyboard language | Setting (existing `keyboard_model_switching.enabled`) | Dictation › Advanced |
| Terminal paste, Share shortcut (Windows) | Settings (existing `paste_with_shift`, `share_dictation_shortcut`) | Dictation › Advanced › Output |
| Second / more / terminal-paste / typed-text shortcuts | Graduated; a shortcut is off while unbound | Dictation's shortcut rows (macOS shows the extra shortcuts; Linux and Windows the second one) |
| Failure notices, live chapters, vocabulary biasing, mouse shortcuts, return to app, smart spacing, subtitle export, sided modifier chords, tap or hold, double-tap on a lone modifier, Parakeet Ultra, special keys alone, section key alone, history stats, history multi-select, recent dictations, stop when call ends, more call apps, movable meeting capsules, meeting notes vocabulary, GNOME clipboard restore, GNOME meeting capsules, GNOME Annotate, meeting finished event, meeting import, descriptive meeting file names, meeting notes file, expire meeting audio, follow live transcript, output fixes, picker sort and filter, model in use first, only downloaded models, model in place, restore shortcut default, release notes link, pause from menu bar, print-only dictation, OpenCode in WSL, transcribe files, API processing, API timestamps, switch model with app | Graduated | Always on where the OS has it |
| Settings search | Removed | The panes are short enough without it |
| Benchmark details | Removed | — |
| Skip pauses | Removed | — |
| Notch indicator | Removed | The speech bubble keeps its place at the top; growing out of the notch hid it |
| Screenshots (and its Captures pane, `hex screenshot`) | Removed | The assistant sees the screen itself; macOS has its own screenshot keys. `screenshots.*` stays in the file, unused |
| Annotate replay | Removed | `capture.replay_hotkey` stays in the file, unused |
| Voice Filter | Removed | Only an installed driver's Uninstall remains |
| Playground | No switch | Dev builds only: ⌘⇧D or `hex window story` |

The managed-preferences key `DisableLabs` locks nothing now; `DisableMeetings`
still locks Meetings.

## Migration

`settings.json` from before keeps loading (`AppSettings::migrate_retired_labs`,
run once in `read_saved_from`):

- A file with a `labs` object and no `preferences` object has its `labs`
  values carried into `preferences` (a file that already has `preferences`
  ignores a stale `labs`). The two-part labs fold into one choice: Copy only
  wins over Keep on clipboard; a theme or a light only counts while its switch
  was on; minutes and seconds outside the offered choices read as off.
- Shortcuts whose lab was off are unbound (they would start working now):
  `second_dictation_hotkey`, `extra_dictation_hotkeys`, `typed_text_hotkey`,
  `terminal_paste_hotkey`.
- On macOS a bound second dictation shortcut becomes the first extra
  dictation shortcut.
- `labs` is never written again; the file is rewritten without it.
- Values the user never chose keep what the old file said: an existing user
  keeps Live text off if it was off, even though new installs have it on.

`hex settings` rejects `labs.*` keys with a message pointing at
`hex settings get preferences`; `hex labs` and `hex experiments` say Labs is
gone. `hex://labs…` links open the Dictation pane.
