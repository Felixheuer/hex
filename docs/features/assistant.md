# Assistant

The assistant (macOS) is a normal Hex feature, **on by default** in this
fork: it left Labs, where it was `labs.assistant` (off). Its switch is
`assistant.enabled` (`AppSettings.assistant.enabled`, default `true`): the
row at the top of Chats › Settings, the off card's Turn on, or `hex settings
set assistant.enabled false`. `crate::assistant::enabled` is
`cfg!(target_os = "macos") && settings.assistant.enabled`, and
`assistant::ready` adds `assistant.setup_done`. While it's off, the Action
shortcut, Stop all tasks, the overlays, the scheduler, the menu bar items,
and the onboarding's assistant pages are inert; Chats stays in the sidebar
and offers to turn it back on. Turning it on while setup isn't done opens
the assistant's onboarding (`chats::open_setup`), and an Action shortcut
another shortcut took meanwhile is unbound (`repair_action_shortcut`). An
older `settings.json` with `labs.assistant` still loads; the key is ignored
like other retired Labs switches. `hex labs on assistant` says it graduated
and how to turn it off.

```ts
Assistant           Toggle (settings.json, hex settings)     On: what it does
└── Assistant       assistant.enabled (default on) + assistant.action_hotkey (macOS)
                                         hold the Action shortcut (right ⌘ alone by default) and say anything;
                                         Hex decides to answer, act, or start a background job; double-tap for the task list;
                                         after an answer Hex listens for a follow-up (conversation_seconds), and speaking over
                                         an answer interrupts it on headphones (barge_in);
                                         Escape stops Here tasks and speech; ⌃⌥Esc (assistant.stop_all_hotkey) stops every task;
                                         General shows both recorders while on; Chats is in the sidebar either way
```

## The Action shortcut and Stop all tasks

While the assistant is on, General shows an "Action shortcut" recorder under
Shortcut (`assistant.action_hotkey`, right Command alone by default,
clearable, with Restore default; also `hex settings set
assistant.action_hotkey right-command`) and a "Stop all tasks" recorder
under it (`assistant.stop_all_hotkey`, ⌃⌥Esc by default, clearable, with
Restore default since Escape ends the recorder; also `hex settings set
assistant.stop_all_hotkey control+option+escape`). The gates are
`AppSettings::active_action_hotkey` and `active_stop_all_hotkey`: the
assistant enabled (`assistant::enabled`), bound, and sharing no key with
another active shortcut (Stop all also never ⌘⌥Esc, Force Quit, and always
with a key); only then does `RuntimeHotkeys::action` or `stop_all` carry it,
the listener (`recognition::Hotkeys`) run a `Shortcut::Action` machine, and
the event tap swallow Stop all. Off or unbound, nothing is matched,
suppressed, or checked for either, and no key or Escape is taken. Recording
another shortcut refuses their keys while the assistant is on, and they
refuse theirs; one that overlaps when the assistant turns on or settings
load (the dictation shortcut moved onto right Command while it was off) is
unbound (`repair_action_shortcut`).

The machine is the dictation shortcut's (`DictationHotkey`), always in
Hold with double-tap on and `set_double_tap_gesture`, whatever the
dictation shortcut's mode. A press starts a capture at once, with the
dictation HUD, the provisional start tone, chord and short-tap discards, and
the chord breaker; right Command with another key (⌘C) is the app's chord
and discards it, and the key reaches the app. The release finishes it as
`TranscriptionTarget::Assistant`: processed like a paste (vocabulary, modes,
replacements), then handed to the question, never pasted, copied, kept for
Paste last dictation, or saved to History (the conversation keeps it). The
gestures (`recognition::ask`):

```ts
Action shortcut (assistant::ready)
├── Hold, say anything -> Request { conversation:
│   assistant::focus(follow_up_seconds), origin: ActionShortcut, context },
│   the context read by computer::context_async once the hold is real; focus
│   is a task waiting on a question (NeedsInput) first, then a running Here
│   task, then a recent one, so a hold answers or steers (never cancelled
│   here); the brain answers, acts here, or escalates to a background job
├── Optional, never needed: tap, then hold (second press within 300 ms of
│   the tap's release, held past the 300 ms hold threshold;
│   DictationHotkey::follows_tap) forces a Background task: Request {
│   conversation: None, origin: Background, context: default }
├── Here words opening "im Hintergrund", "in the background", "nebenbei",
│   "parallel", or "Hintergrund:" (ask::spoken_background; whole words, any
│   case, punctuation around them ignored) -> Background, without them
├── Double tap (both presses shorter than 300 ms, the second within 300 ms
│   of the first's release) -> HotkeyAction::DoubleTap: the second capture
│   is discarded, the task list shows or hides (ask::set_toggle_tasks), and
│   the success sound plays; nothing locks
├── A hold that becomes real -> assistant::service::prewarm(lane), once
├── Words for Here -> speech::stop() before submitting; a press, a tap, or
│   silence leaves the answer being read aloud
└── Not ready (setup unfinished) -> nothing recorded; a hold calls
    ask::set_open_setup's hook (onboarding at its saved page, HUD note)
```

A silent, short, or discarded capture asks nothing. A hold followed by a
quick tap never pairs. The machine that starts a capture owns it, so
dictation and the Action shortcut stand down while the other records; `hex
dictation stop` refuses while a question records (`cancel` still discards
it); a microphone loss mid-question fails it instead of salvaging a copy.
Stop all tasks' press runs `assistant::service::stop_all()` and
`speech::stop()` and plays the cancel sound, once per press (`Asker::key`).

## Conversation mode

Conversation mode (`recognition::conversation`, a pure state machine, and
`recognition::talk`, which carries it out): after a Here answer, or a
question a Here task asks (`Event::Answer` with `Origin::ActionShortcut`,
`Event::Question` for a running Here conversation), once the reading ends
(or never starts within 1.5 s, or `speak_answers` is off) the listener opens
an ordinary capture without a tone and sends
`DictationIndicatorEvent::Listening` for `assistant.conversation_seconds`
(8 s by default; 0 turns it off). Speech (eight 20 ms frames by the
dictation pipeline's `SpeechFrames` test, two quiet frames forgiven) makes it
a question (`Started`, `Asking`, the desktop read, `prewarm(Here)`); 900 ms
without speech (or 60 s) finishes it through the Action shortcut's finish
into the same conversation. Nothing in time closes it quietly. With
`assistant.barge_in` (on by default) the capture opens as Hex starts
reading: on headphones or AirPods (`audio::output_route` reads Bluetooth,
the headphone jack, and Apple silicon's headphone output) 240 ms of speech
stops the reading (`speech::stop()`) and becomes the follow-up; on speakers,
where the microphone hears Hex's voice, only speech 6 dB louder than the
answer's own playback level (`speech::level()`) does, and while that level is
unknown (its stub returns `None`) barge-in is off on speakers. Any key going
down or modifier changing ends it before the shortcut it may be can start its
own capture; so do a dictation, a command's capture, a pause, a meeting
recording (`meeting::watcher::recording`), or the setup unfinished. A
microphone loss never salvages its clip. Checks:
`an_answer_then_listening_then_a_follow_up`, `listening_times_out_quietly`,
`shown_answers_and_conversation_mode_off`,
`a_key_or_another_recording_ends_it`, `barge_in_on_headphones`,
`barge_in_on_speakers_needs_a_voice_over_the_answer`,
`a_newer_answer_moves_it_along`,
`without_barge_in_nothing_listens_under_an_answer`, and
`audio_is_heard_in_frames` in
[recognition/conversation.rs](../../src/recognition/conversation.rs);
`only_here_answers_and_questions_are_listened_after` and
`keys_end_conversation_mode` in
[recognition/talk.rs](../../src/recognition/talk.rs). Not yet observed: a
real microphone, the overlays' drawing of `Listening`, or barge-in through
real speakers.

## Whether the shortcuts can work

Whether the shortcuts can work (macOS, `shortcut_conflicts` and
`recognition::shortcut_probe`). `shortcut_conflicts::conflicts(slot,
settings)` checks a saved shortcut against what can take its keys first,
each `Taken` (a saved setting names the same keys) or `Possible` (on or
running, and commonly uses such keys):

```ts
conflicts(slot, settings) -> Vec<Conflict { source, certainty, name, description, fix }>
├── macOS keyboard shortcuts (com.apple.symbolichotkeys)  // Taken; unsaved IDs on with defaults
│   └── Spotlight ⌘ Space, input sources ⌃ Space, Mission Control, screenshots, …; others as "a macOS keyboard shortcut"
├── 🌐 alone vs "Press 🌐 key to" (HIToolbox AppleFnUsageType)   // Taken; unsaved -> Possible
├── Apple Dictation on + a lone ⌘, ⌃, or 🌐 pressed twice     // Possible: its shortcut isn't readable
├── Siri on + a lone ⌘ pressed twice, or ⌘/⌥/🌐 Space held    // Possible; HotkeyTag 0 reads as off
├── Raycast running: raycastGlobalHotkey ("Option-49")       // Taken when it matches; unreadable -> Possible on Space
├── Karabiner-Elements installed: karabiner.json's selected profile
│   ├── a simple modification of one of the keys            // Taken
│   └── a complex rule from one of them                     // Possible
└── running Superwhisper, Wispr Flow, MacWhisper, Spokenly (any shortcut),
    Alfred (a chord on Space), BetterTouchTool, Hammerspoon, Keyboard Maestro (any)  // Possible
```

`shortcut_conflicts::cached` reads again at most every 3 s, for Settings.
While the assistant is on, General shows each conflict under the dictation and
Action shortcut rows (red when taken, muted when possible) with where it is
fixed (Keyboard settings, Siri settings, or the app) and "Choose another
shortcut"; no conflict, or the assistant off, draws nothing new. The screenshot
Lab's check (`screenshots::system_shortcut`) reads the same symbolic hot
keys. `shortcut_probe::start(slot, wait)` asks for a press: while it runs
the listener hands every edge to it, not the shortcut machines (nothing
records), and the HID tap reports every key press
(`suppression::set_probing`), so it tells a press and release that reached
Hex with exactly the keys and sides (`Works { held }`) from a key the
keyboard saw but Hex's tap never got (`Swallowed`), the wrong modifiers or
side (`WrongKeys`), a missing release (`NoRelease`), and nothing
(`NothingReceived`). It refuses (`Unavailable`) without the listener, while
Settings records a shortcut, or while a recording runs. Checks: the six
fixture-plist tests in
[shortcut_conflicts/tests.rs](../../src/shortcut_conflicts/tests.rs) and the
four in [recognition/shortcut_probe.rs](../../src/recognition/shortcut_probe.rs).
Not yet observed: Raycast's and Siri's saved formats on a Mac that has them
set (neither is set on the development Mac), a real probe, or the notes'
rendering.

## Escape

Escape belongs to the assistant only while a Here task
(`Origin::ActionShortcut`) runs or waits, or an answer is read aloud
(`speech::speaking()`), and nothing records: the tap then keeps it from the
app (`guards::assistant_claims_escape`), and it runs `assistant::cancel` for
each Here task and `speech::stop()`. Background and Chats tasks keep running
(each has its own stop, and Stop all tasks stops them all). Checks:
`a_hold_asks_here`, `a_tap_then_a_hold_asks_in_the_background`,
`a_double_tap_is_a_gesture`, `a_tap_alone_asks_nothing`,
`right_command_with_a_key_is_a_normal_chord`,
`the_action_shortcut_and_dictation_never_share_a_capture`, and
`without_its_machine_right_command_does_nothing` in
[recognition/hotkeys.rs](../../src/recognition/hotkeys.rs);
`a_hold_asks_with_its_lane_settled_at_the_press`,
`the_desktop_is_read_only_once_the_hold_is_real`, `no_words_ask_nothing`,
`the_transcript_submits_and_a_dropped_reply_asks_nothing`,
`spoken_openings_route_to_the_background`,
`words_that_merely_contain_an_opening_stay_here`,
`a_spoken_opening_asks_a_background_task`, `escape_stops_only_here_tasks`,
`the_stop_all_shortcut_presses_once_and_only_while_bound`,
`a_double_tap_calls_the_task_list_toggle`, and `only_a_hold_opens_the_setup`
in [recognition/ask.rs](../../src/recognition/ask.rs);
`escape_is_the_assistants_only_while_it_is_busy` in
[recognition/guards.rs](../../src/recognition/guards.rs);
`an_assistant_question_is_only_answered_never_pasted_or_kept` in
[parakeet.rs](../../src/parakeet.rs);
`the_action_shortcut_needs_the_assistant_and_its_own_keys` and
`stop_all_needs_the_assistant_and_its_own_keys` in
[app_settings.rs](../../src/app_settings.rs); `stop_all_is_a_shortcut_setting`
in [control/settings.rs](../../src/control/settings.rs). Not yet observed: a
real press on a Mac, the recorder rows' rendering (macOS only, so no golden
changes), Escape against a running question, or ⌃⌥Esc reaching Hex past
macOS.

## Chats

The assistant's window (`rune_ui/chats`, macOS): Chats is in the sidebar
after History on macOS (`Pane::Chats`, `chats::available`), whether or not
the assistant is on. While it's off, Conversations shows a calm "The
assistant is off" card with Turn on (`chats/switch.rs`), the header drops
New chat and Run setup…, and a running onboarding closes. Its header switches
Conversations and Settings, and shows the agent's name, Agent mode
(`assistant::set_agent_mode`), and New chat. Conversations is a list and
detail: the conversations grouped Today, Yesterday, Previous 7 days, and
Older, searchable, each with how it was asked (microphone, Agent mode,
keyboard) or, while it runs, a dot in its task's colour
(`assistant_overlay::task_colour`) and its latest step; running and queued
tasks sit in a strip above with Stop each and Stop all. The transcript
shows questions as bubbles, answers as Markdown (`rune-kit`'s `markdown`
feature; streaming while `AnswerDelta`s arrive), what the assistant did as
muted steps (a run longer than four folds to its last step), screenshots as
thumbnails that open the file, and errors; Delete chat… asks first. The
composer sends `Request { origin: Chat, conversation: <shown> }` on Return
(Shift-Return starts a line), and its button stops the shown request while
it runs. The conversation showing is named with `assistant::set_focus`, and
cleared when the pane, view, or window goes. Every `assistant::subscribe`
event wakes the window. `hex://chats?id=<conversation>`,
`hex://settings/chats`, the menu bar, and the HUD's task stack and "Open in
Chats" (`assistant_overlay::set_open_chats` → `macos_rune::open_chat`) open
it. Settings: the Assistant switch first (`assistant.enabled`, "Hold right ⌘ to
ask, double-tap for your tasks"), then the agent (and whether each is ready), its model, the Claude
API key (Keychain only), Read answers aloud with its voice and Test voice,
Operate apps, the real pointer and keyboard (with its warning), Send a
screenshot, the follow-up window, Tasks at once (1 to 64), and Run setup….

## Onboarding

The assistant's onboarding (`chats/setup.rs`, drawn by `chats/sheet.rs` in
the first run's type, card, and motion over the whole window) opens when
the assistant is turned on (Settings' switch or the off card's Turn on)
while `assistant.setup_done` is false, after the voice first run on a fresh
install, and from Run setup…
or Chats' notice. Pages: Meet your assistant; Choose who answers (the
agents `backends::detect` found, Claude Code by default, each with its fix:
install, sign in, start OpenCode, or the API key); Hex's Microphone,
Accessibility, and Screen Recording, each Allow opening its System Settings
page with Hex's guide and waiting until macOS reports it on (read every
1.5 s); agent-desktop, installed with Homebrew if missing, and its two
permissions; the self-tests (`backends::self_tests`), run on arrival, with
each failure's fix; the Action and dictation shortcuts, with what else takes
them (`shortcut_conflicts::conflicts`, read again every 3 s, each with where
it's fixed and, for the Action shortcut, Choose another…) and a live press
test of each (`recognition::shortcut_probe`; ✓ only when the press and its
release reached Hex, and a swallowed press names the app that took it), the
page waiting for a free Action shortcut that works (`chats/shortcut.rs`
adapts both); then the try-it: hold the Action shortcut and ask, with the
double-tap Agent mode hint, waiting for one real spoken question, answered. A row shows ✓ only after its check saw
it work. Continue waits for its page; Later moves on and marks it unfinished
(amber in the progress marks). Finish waits until the chosen agent answers
and can use agent-desktop, then sets `assistant.setup_done`. Menu bar: Agent
Mode (checked while on) and Chats… show only while the assistant is on. Checks:
the tests in [rune_ui/chats/tests.rs](../../src/rune_ui/chats/tests.rs),
`reads_chats_links` in [deep_link.rs](../../src/rune_ui/deep_link.rs), and
`the_assistant_rows_start_hidden_and_do_something` in
[status_item.rs](../../src/macos_rune/status_item.rs); scenes `mac-chats*`
and `mac-assistant-setup*` (macOS only, so no golden changes). Not yet
observed: the live service, real grants, or a real question on a Mac.

## Agents on Linux and Windows

The agent backends (Claude Code, Codex, OpenCode, agent-desktop,
agent-browser) run the same way on every OS; what differs:

- **Process trees** (`backends/run.rs` `own_group`/`Tree`): Unix puts each
  agent in its own process group (on Linux a helper Hex waits for also gets
  `PR_SET_PDEATHSIG`); Windows puts it in a Job Object (shared with
  `personal_commands/process.rs`) that Cancel, steering, and a failure end
  at once, and that ends everything if Hex dies. A normal exit releases the
  job, so a daemon an agent left on purpose (agent-browser's) keeps running,
  as on Unix.
- **Finding programs** (`backends/search.rs`): Linux asks the login shell
  (bash, else sh, when `$SHELL` is unset) and adds Linuxbrew, pnpm, snaps,
  and `~/.npm-global/bin`. Windows has no shell to ask: the PATH comes from
  the registry (machine, then user, `%VAR%` expanded), plus npm's
  `%APPDATA%\npm`, WinGet's links, `%USERPROFILE%\.local\bin`, Bun, Scoop,
  pnpm, Volta, fnm, nvm-windows, `%ProgramFiles%\nodejs`, and System32.
  `name.exe` beats an npm `name.cmd` shim, and a shim runs as the
  `node.exe <script>` (or `.exe`) it wraps (`search::launch`), in Hex's own
  commands and in the MCP server configs given to agents, because batch
  files can't take the newlines and quotes in the agents' arguments. MCP
  servers get `SystemRoot`, `PATHEXT`, `ComSpec`, `APPDATA`, `LOCALAPPDATA`,
  `USERPROFILE`, and the other variables Windows programs need besides PATH.
- **Installs** (`backends/install.rs`, `Os`): macOS is unchanged
  (Homebrew, npm, script). Linux uses Claude Code's native script, then npm
  (into `~/.local` when npm is the system's), then the project's script,
  then a Linuxbrew formula. Windows uses `winget install
  Anthropic.ClaudeCode`, else npm (`@openai/codex`, `@opencode/cli`,
  `agent-desktop`, `agent-browser`), else the project's PowerShell script.
  Lightpanda has no Windows build: its install and self-test are hidden
  there. "Copy install command" quotes for PowerShell on Windows and joins
  with `;`. Sign-in opens Terminal on macOS, the desktop's terminal on Linux
  (`xdg-terminal-exec`, `x-terminal-emulator`, GNOME Terminal, Konsole, …),
  and a console window on Windows.
- **Sandboxes**: Claude Code's job shell is sandboxed on macOS and, with
  bubblewrap and socat installed, on Linux. Without a sandbox (Windows, or
  Linux without them) a job gets no shell (`Bash` denied, file tools still
  scoped to the workspace), and the self-test says so. Windows permission
  rules use Claude Code's `//c/Users/...` form. Codex uses its unelevated
  Windows sandbox.
- **Liveness and time**: process start times come from `/proc` on Linux and
  `GetProcessTimes` on Windows; liveness from `OpenProcess` +
  `GetExitCodeProcess` on Windows, so conversations no longer look
  interrupted there. Local time uses `localtime_s` on Windows
  (`schedules::local_tm`). Job folders live in `app_paths::document_dir()`
  and their names avoid what Windows refuses.
- **Guard**: on Linux and Windows Ctrl counts as ⌘ for the consequential
  shortcuts, and Delete / Shift+Delete in Explorer, Nautilus, Dolphin (and
  the other file managers), Ctrl+Enter in Outlook or Thunderbird count too.
  On Linux a desktop-permissions failure checks the AT-SPI bus
  (`org.a11y.Bus`) and asks for at-spi2-core when it's missing.

Not yet observed on a real Linux desktop or Windows PC: installs, sign-in
windows, the shim launch, job objects, or the agents' sandboxes.
