# Bench: Agent-Desk's local benchmark

`agent-desk bench` asks about 25 typical tasks of the running Agent-Desk Dev, checks each
result by machine, and compares the run with the one before
(docs/roadmap-agent-desk.md, G). `scripts/self-improve.sh` wraps it in the
self-improvement loop (I).

## What runs where

| Part | Where |
| --- | --- |
| Tasks | `bench/tasks/*.toml`, one per file, run in file-name order |
| Test recipients, reading-task values | `bench/config.toml` (not committed; copy `bench/config.example.toml`) |
| Results | `bench/results/<version>-<YYYYMMDD-HHMMSS>.json` (UTC), committed |
| Temp folders | `~/Documents/Agent-Desk/bench/<run>/<task>-<lang>/`, emptied before each task |
| Findings | `bench/findings.md`, written by the loop |
| Code | `src/bench.rs` (CLI), `src/bench/{task,check,run,results,live}.rs` |

The bench talks to the app only through its owner-only local API
(`local-api.json` in the target's data folder): `status` for the version,
and `assistant-submit`, which asks the assistant as a hold of the Action
shortcut would, but with **no window in front**, so the task never reads or
acts on the person's front window. It follows each task in the
assistant's conversation files (`<data>/assistant/conversations/<id>.json`)
and audit log (`<data>/assistant/audit/<id>.jsonl`), and stops a task that
times out the way `agent-desk assistant stop-all` does. The desktop is only read:
`lsappinfo` for the front app, and Cua Driver's (`guard::Reader`) `desktop_list_apps` and
`list-windows`.

`--target dev` (the default) uses `~/Library/Application Support/agent-desk-dev`;
`--target live` must be named to use Agent-Desk's `agent-desk` folder. The bench never
starts either app: when the target isn't running it says so and exits 3.

## Usage

```sh
agent-desk bench run --dry-run                 # validate tasks, config, checks; ask nothing
agent-desk bench run                           # every task, in German, against Agent-Desk Dev
agent-desk bench run --lang both --filter 'web|supplier'
agent-desk bench run --allow-front             # also tasks that bring an app forward (nobody at the Mac)
agent-desk bench run --target live             # only when named
agent-desk bench report [FILE] [--against FILE] [--json] [--fail-on-regression]
agent-desk bench list
```

From a checkout, `cargo run --bin agent-desk -- bench run --dry-run` works too: the
control commands never open the app. `run` exits 0 when the run is not worse
than the previous run of the same target, 1 when it is (a task that passed
fails now, or fewer pass), 2 when a task file breaks the rules, and 3 when
the target isn't running.

Each task records: status (`passed`, `failed`, `timeout`, `skipped`,
`error`), time to the first answer, total time (until it and every job it
started ended), steps (tool calls), the lane it ran in, every check's
verdict, and the answer (shortened). The table marks regressions ▼ and fixes
▲.

## Task files

```toml
id = "supplier-csv"                  # lowercase, digits, dashes; unique
title = "A supplier list as CSV"
covers = "G. Build a supplier list"  # the roadmap item
lane = "job"                         # quick | job | any (checked)
timeout_s = 900                      # 5..=1800
after = "quick-math"                 # optional: continue that task's conversation

[safety]
messages = "none"                    # none | draft
front = "keep"                       # keep (checked) | may_change (needs --allow-front)

[text]                               # both languages; {key} placeholders
de = "… als CSV-Datei im Ordner {temp_dir}."
en = "… as a CSV file in the folder {temp_dir}."

[background]                         # optional: a job asked first; the task comes lead_ms later
de = "…"
en = "…"
lead_ms = 5000

[[seed]]                             # optional: files put in the temp folder first
name = "bericht.txt"
content = "…"

[[precondition]]                     # optional, read-only: skip the task when it doesn't hold
kind = "app_running"
app = "{ocr_app}"

[[check]]
kind = "file"
glob = "*.csv"
contains = "(?i)https?://"
```

Placeholders: `{temp_dir}` and `{run}` come from the runner; every other key
from `bench/config.toml`. A task whose key is missing is skipped, never
guessed. In patterns, values are matched literally, except keys ending in
`_re`, which are regular expressions.

Checks (`kind`): `answered`, `answer_matches {pattern, lang?}`,
`answer_language` (German in, German out), `answer_within_ms {ms}`,
`max_steps {steps}`, `app_running {app}`, `front_app {app, is?}`,
`window {app, title?, visible?, present?}`, `file {glob, contains?, min?,
in = temp|workspace}`, `browser_url {pattern, min?}`, `tool_used {pattern}`,
`tool_not_used {pattern}`, `plan {min_steps?, all_done?}` (the job's
`plan.md`), `while_running` (with `[background]`), `same_conversation`
(with `after`). Every task also gets its lane check, and with `front =
"keep"` a check that the front app never changed while it ran.

### Rules `--dry-run` enforces

- Message tasks are `messages = "draft"`, address `{test_contact}` or
  `{test_email}` in both languages, and carry a `tool_not_used` check
  against sending. No task names a phone number or an email address
  literally, and none names a recipient while saying `messages = "none"`.
- Spoken text names no path outside the run's temp folder (`~/`, `/Users/`,
  …); a `file` check in the temp folder needs `{temp_dir}` in the text.
- Preconditions only read the desktop; patterns compile; ids are unique;
  `after` names an earlier task; unknown fields are refused.

## The tasks

| # | id | covers |
| --- | --- | --- |
| 01 | quick-math | latency of a simple question |
| 02 | follow-up | follow-up in the same conversation |
| 03 | weekday | quick fact |
| 04 | language-explain | German in, German out |
| 05 | translate | language |
| 06 | running-apps | reading the desktop without acting |
| 07 | open-app-parked | open an app in the background |
| 08 | switch-to-app | switch to an app (`--allow-front`) |
| 09 | park-app | park an app (`--allow-front`) |
| 10 | other-space | a window on another Space |
| 11 | whatsapp-draft | WhatsApp draft to the test contact, never sent |
| 12 | email-draft | email draft to the test address, never sent |
| 13 | web-three-browsers | web search in three browsers |
| 14 | web-quick-answer | a quick turn that browses |
| 15 | supplier-csv | supplier list as CSV |
| 16 | supplier-numbers | supplier list as Numbers |
| 17 | note | create a note |
| 18 | text-file | write a file |
| 19 | find-file-reveal | find a file and show it in Finder (`--allow-front`) |
| 20 | summarize-file | read a file |
| 21 | calendar-read | read a calendar appointment |
| 22 | chrome-tab | read a tab of the person's Chrome |
| 23 | ocr-window | read a window as text (Citrix) |
| 24 | answer-during-job | answer while a job runs |
| 25 | research-report | a job with a plan and a result file |

## The self-improvement loop

`scripts/self-improve.sh` runs only when the person allows it, because
starting Agent-Desk Dev quits Agent-Desk: `--now`, or `--when idle[=MIN]` (no keyboard or
mouse input for MIN minutes, 20 by default, from `HIDIdleTime`). It installs
Dev (`scripts/install-fork.sh`), opens it in the background, runs the
bench, and starts Claude Code with `bench/self-improve.md`: findings into
`bench/findings.md`, fixes in a worktree with filtered tests only, a new Dev
build, a rerun, and a PR when the bench isn't worse. `--bench-only` stops
after the bench; afterwards it reopens Agent-Desk if it ran before (`--keep-dev`
leaves Dev). `--dry-run` only validates.
