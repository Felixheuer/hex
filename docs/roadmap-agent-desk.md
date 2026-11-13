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

- **Native action service in Agent-Desk:** an `AXUIElement` server inside the process, with
  cached element trees per window and diff updates. It replaces one agent-desktop process
  per call. Target: under 50 ms per click, type or read of a known element.
- **Every app, discovered, not listed:** no per-app recipes or app lists. The agent asks
  any installed app what it offers with `hex_app_capabilities` (read from the app itself
  in about 25 ms, cached per app version): its scripting dictionary (`sdef`, summarized to
  classes, properties, elements and commands), the URL schemes and documents its
  `Info.plist` declares, what its windows are made of (native, Electron, Catalyst, iPhone
  or iPad), the command-line tools it ships, and the links it opens by default
  (LaunchServices). `hex_app_action` then acts in any app by `script` (AppleScript inside
  `tell application id …`, which reads and changes the app's data without its window, on
  any desktop) or `open_url` (any URL the app takes). One rule for all apps, fastest
  first: scripting, then a URL scheme, then Accessibility in its parked window, then
  `hex_read_window`; an unreachable window means the next method, never asking the person
  to bring it over. The guard checks both tools like any action (blocked apps, autonomy
  for scripted sends and deletes).
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

Built as `agent-desk bench` (until the rename to `agent-desk`): 25 tasks in `bench/tasks/`; see
[docs/bench.md](bench.md).

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

## I. Self-improvement loop ("test yourself")

Built as `scripts/self-improve.sh` with the prompt `bench/self-improve.md`; it runs only
with `--now` or `--when idle[=MIN]` (docs/bench.md).

The goal: "Open Agent-Desk in VS Code and the console, test yourself, find errors and gaps,
note them, fix them, start your new version, and keep improving."

- **Loop runner outside the app:** a Claude Code session in Ghostty or VS Code, started by
  Agent-Desk itself as a job or by the person. It works in `~/dev/desktop/ai-desktop` on
  its own branch and in its own worktree.
- **Steps:**
  1. Install the Dev build with `scripts/install-fork.sh`.
  2. Run `agent-desk bench` against Dev through Dev's local API. No real input on the
     person's windows, and messages only as drafts or to test recipients.
  3. Turn each failure into a note in `bench/findings.md`: task, expectation, what
     happened, logs.
  4. Fix: filtered tests only (never the full suite, never audio tests; see CLAUDE.md),
     then a new Dev build.
  5. Run the benchmark again. If it is better or equal, open a PR and merge. A release to
     Live only after the run passes without a regression.
- **Limits:**
  - Starting Dev quits Live, because they share the shortcut and microphone. So the loop
    runs when the person isn't using the Mac (at night, or when "testing" is said), or
    the benchmark drives Dev without the global shortcut.
  - Every round writes a short changelog of what it found, fixed and measured.

## H. Product and identity (after the round)

- **Rename Agent-Desk to Agent-Desk:**
  - Display name, CLI `agent-desk`, URL scheme, data folder (migrated) and docs.
  - Bundle ID `fheuer.agentdesk` and `fheuer.agentdesk.dev`, with a one-time
    permission repair.
  - The LICENSE copyright and the upstream sync path stay.
- **Website and release channel** at ai.sms-pilot.de: Agent-Desk branding, and the
  updater moves the bundle once.

## Order and parallelism (round two)

| Agent | Scope (files) | Content |
|---|---|---|
| 1 Actions | `src/assistant/actions/**` (new), guard integration | A: action service, app discovery (`hex_app_capabilities`), `hex_app_action`, background operation, OCR |
| 2 Knowledge | `src/assistant/knowledge/**` (new) | B: personal index, `hex_lookup`, own Chrome, memory graph |
| 3 Speed | `src/assistant/agent*.rs`, `provider/**`, `service/**` | C: two models, warm session, speculative start, compact prompts |
| 4 Benchmark | `bench/**` (new), CLI | G: tasks, measurement, comparison |

Then: integrate, run the benchmark, release. Then H, the rename, as its own pass.
