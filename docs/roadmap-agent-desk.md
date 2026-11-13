# Agent-Desk: second round

The goal: the AI desktop that operates the whole Mac faster and more reliably than a person
would. It runs from voice and the overlay alone, knows everything about the computer, and
proves each improvement with measurements.

Status after round one: the agent sees the desktop state, opens apps and parks them as live
minis, browsers are pre-warmed, the window is redesigned without Labs, and the overlay is
the main interface.

## Principles

1. **Act directly, click only as a fallback.** For every action, the fastest way comes
   first: an app's URL scheme or deep link, then AppleScript, Shortcuts, or a CLI, then the
   Accessibility API, and only then clicking on screen pixels.
2. **Know first, then act.** Desktop state, personal data and memory are available in
   milliseconds. The model never has to search for what the system already knows.
3. **Small things right away, big things in the background.** Under two seconds for
   "open", "switch" and "what is open". Jobs only for multi-step work.
4. **The person sees only what matters:** live minis, a frame while acting, questions, and
   results. No paperwork for small things.
5. **Measured, not guessed.** Every release runs the task benchmark (section G) and
   reports its speed and success rate.

## A. Fast action layer ("actions")

- **Native action service in Hex:** an `AXUIElement` server inside the process, with
  cached element trees per window and diff updates. It replaces one agent-desktop process
  per call. Target: under 50 ms per click, type or read of a known element.
- **Deep links and app recipes:** a registry of fast paths per app:
  - WhatsApp: `whatsapp://send?phone=…&text=…`, opens the chat.
  - Slack: `slack://channel?team=…&id=…`.
  - Mail: `mailto:` plus AppleScript to create a draft.
  - Messages: AppleScript `send`.
  - Calendar and Reminders: EventKit.
  - Notes: AppleScript.
  - Finder: `open -R`.
  - System Settings: `x-apple.systempreferences:` panes.
  - Music and Spotify: AppleScript.
  - VS Code: `code` CLI.
  - Ghostty or Terminal: AppleScript, then `claude`.
  - Shortcuts: the `shortcuts run` CLI.

  The recipes live as data (TOML) in the app. The agent gets the `hex_app_action` tool
  (app, intent, parameters), and the guard checks the recipes like any other action.
- **Background operation for every app:** windows are claimed, parked off-screen and
  driven through AX and window-specific events, without bringing the app to the front.
  Where an app refuses that, fall back to its own window on a separate Space.
- **Screen text without the model:** the Vision framework's `VNRecognizeTextRequest` on
  the window picture, cached per frame, for apps without AX such as Citrix and games. The
  `hex_read_window` tool returns text with coordinates.

## B. Knowing everything ("knowledge")

- **Desktop state service** (from round one) as a resident cache: apps, windows, Spaces,
  front app, focus, displays, screen lock, network, audio. It updates from NSWorkspace and
  AX notifications, with no polling.
- **Personal index**, read-only, all local. The `hex_lookup` tool takes a query and a
  scope:
  - Files: Spotlight (`mdfind`, `NSMetadataQuery`) and recently used files.
  - Contacts: the Contacts framework, including phone, email, WhatsApp number and
    company.
  - Calendar and reminders: EventKit.
  - Mail: index of the Mail app (subject, sender, date).
  - Chrome: open tabs (AppleScript) and history (SQLite copy).
  - Messages and WhatsApp: only chat names and dates, never content without a request.
- **The person's real Chrome:** read and drive the tab the person means ("my open tab"),
  through AppleScript, JavaScript in the tab, or CDP when Chrome runs with debugging. No
  hidden copy when the person means their own tab.
- **Memory as a graph:** people, companies, projects, accounts and channels with
  relations (Michael Schwarz → WhatsApp +49…; Kalibermarkt → staging.kalibermarkt.de →
  repo …). Built automatically from tasks and searchable in every turn's context.

## C. Speed of thinking

- **Two models:** a fast model (Haiku) for routing, short answers and small actions; the
  strong model for jobs and hard steps. Escalate automatically when it's uncertain.
- **Warm session:** the Here session stays started (the process runs and its context is
  loaded), so the first word comes under one second after the key is released.
- **Speculative start:** while the person is still speaking, the partial transcript
  already warms what is needed (browser, app, lookup).
- **Fewer round trips:** batched tool calls (snapshot and action in one), short tool
  results (diffs instead of full trees), compact system prompts per lane.

## D. Background like Codex

- **Every app the agent uses:** parked, shown as a live mini in the overlay with
  "Take over", and the frame visible only while it acts.
- **Many tasks in parallel:** each with its own browsers and windows, a wall view showing
  all of them, and status only when asked.
- **Long tasks (up to 7 days):** checkpoints, resume after a restart or lock screen, an
  iPhone channel for questions.

## E. Voice and overlay

- **One gesture:** hold the key and speak. The assistant decides between an answer, an
  action, a job, or a follow-up question.
- **Voice in, voice out:** answers in the person's language, interruptible only with the
  key, music lowered.
- **Overlay complete:** questions, approvals, results with actions (open, show, undo),
  live minis, the wall, and recent results. No window needed.
- **Undo:** for actions that can be reversed (moved file, draft, closed window).

## F. Reliability

- **Self-check after every action:** expected result against desktop state or snapshot,
  automatically repeated or rerouted (deep link → AX → click).
- **Error classes with a strategy:** wait and retry on rate limits or Cloudflare; fall
  back to the person's live view on a captcha or login; reopen when an app hangs;
  permission repair when access is missing.
- **Clean permission flow:** an onboarding check, repair after updates, a status in the
  overlay.

## G. Benchmark and quality

- **A local benchmark run** (`agent-desk bench`) with about 25 typical tasks:
  - Open, switch to and park an app.
  - Draft a WhatsApp message to a test contact without sending.
  - Search the web with three browsers.
  - Build a supplier list as Numbers or Google Sheet.
  - Create a note.
  - Find a file and show it in Finder.
  - Read a calendar appointment.
  - Read a tab of the person's own Chrome.
  - Read a Citrix window as text.
  - Answer a question while a job runs.
- **Measured per task:** time to first reply, total time, success (checked automatically),
  and number of steps.
- **Rules:**
  - Messages go only to test recipients or are only drafted.
  - The run never takes over the person's front windows.
  - Results land in `bench/results/<version>.json` with a comparison to the previous
    version.

## H. Product and identity (after the round)

- **Rename Hex to Agent-Desk:**
  - Display name, CLI `agent-desk`, URL scheme, data folder (migrated) and docs.
  - Bundle ID `fheuer.agentdesk` and `fheuer.agentdesk.dev`, with a one-time
    permission repair.
  - The LICENSE copyright and the upstream sync path stay.
- **Website and release channel** at ai.sms-pilot.de: Agent-Desk branding, and the
  updater moves the bundle once.

## Order and parallelism (round two)

| Agent | Scope (files) | Content |
|---|---|---|
| 1 Actions | `src/assistant/actions/**` (new), guard integration | A: action service, deep-link recipes, `hex_app_action`, background operation, OCR |
| 2 Knowledge | `src/assistant/knowledge/**` (new) | B: personal index, `hex_lookup`, own Chrome, memory graph |
| 3 Speed | `src/assistant/agent*.rs`, `provider/**`, `service/**` | C: two models, warm session, speculative start, compact prompts |
| 4 Benchmark | `bench/**` (new), CLI | G: tasks, measurement, comparison |

Then: integrate, run the benchmark, release. Then H, the rename, as its own pass.
