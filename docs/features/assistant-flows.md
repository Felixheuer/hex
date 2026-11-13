# Assistant flows, as the person meets them

How every assistant flow looks and sounds from the outside, what the code
does for it, and the gaps that remain. Principle: everything works by voice
and on the screen's overlay, without the Hex window; one gesture (hold the
Action shortcut and speak), and the assistant decides the rest.
How overlapping moments resolve is in `src/assistant/interaction.rs`.

Audited on dev @ 9c2f35ff (pilot.15) by reading the code and the
fake-backend service tests, and again after pilot.17 (three read-only
audits: voice, jobs architecture, overlays); "Gap" rows are open.

## Asking

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| Hold, short question about the window in front | HUD while held; answer bubble at the HUD, read aloud | Here task answers from the window (or the window right behind Hex when Hex itself is in front) | `recognition/ask.rs`, `computer::quick_context`, `capture::frontmost_external_app_info` |
| Hold, a job ("mach ein Werbevideo über diese Seite") | Short spoken line („Mach ich …“), frame on | **Fixed:** the agent calls `hex_start_jobs`; the runner starts the job when the run ends (`agent::handoff`). Fallbacks: the `HEX_JOB` marker, and an answer that only *claims* it started (prose) starts the whole request | `agent::handed_on`, `tools.rs` |
| Hold, several independent jobs | One line, several tasks in the list | `hex_start_jobs` with several jobs: the conversation goes on with the first job's own part, the others start as their own conversations | `service/run.rs` Escalate `first_task` |
| Hold with an app/browser request ("öffne einen Browser") | Handed on, never "do it yourself" | Here rule + `routing` name the tool | `agent::system_prompt`, `agent::routing` |
| Hold, "Stopp" / "Halt" / "Sei still" | Hex goes quiet | Escape path; "Stopp alle Aufgaben" = Stop all | `shortcut/spoken_stop.rs` |
| Typed in Chats | Answer in the transcript | Same routing as a hold (Chat lane = Background rules, `routing` applies) | `agent::prepare` |

| Hold while a job's question came up during the hold | Goes where it would have gone | A hold answers the inbox's question only if it waited when the key went down (`Question::asked_at_press`, `service::focus_past_inbox`) | `shortcut.rs` |
| „Abbrechen“ while a job asks „abschicken oder abbrechen?“ | The job gets the answer | A plain stop word goes to the waiting question when nothing is read and no quick question runs (`answers_a_question`); „Stopp alle Aufgaben“ still stops all | `shortcut.rs` |

## While Hex speaks

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| Hold while Hex reads aloud | Hex stops at the press | `interaction::hush` when the capture becomes real | `recognition.rs` |
| Talking over the answer (headphones) | Hex pauses; the words become the next request | barge-in 240 ms → pause; transcript decides | `conversation.rs`, `shortcut/barge_in.rs` |
| Talking over the answer (speakers) | Same, after 500 ms of voice the echo model can't explain | Empty/own-echo transcript resumes the cut sentence | `recognition/echo.rs` |
| Music or another app's audio | No hands-free window, no barge-in | Hold and "Stopp" still work | `recognition/other_audio.rs` |
| After an answer (conversation mode) | Listening ring for `conversation_seconds` | **Fixed:** the window is now a listening state of its own (`interaction::Listening::FollowUp`), so job summaries and inbox questions wait instead of being spoken into it | `interaction::listening`, `talk.rs` |
| Hex's own voice heard back | Nothing | Dropped (`speech::own_echo`) | `shortcut.rs` |
| A barge-in or follow-up whose capture also heard the reading | Only the person's words are asked | `own_echo::without_own_words` lines the transcript's opening up with what Hex read and keeps the rest; the follow-up window restarts its capture once the reading ended (`Talk::reopen`) | `own_echo.rs`, `talk.rs` |
| A hold, a dictation, or Escape while an answer streams | Quiet, and the rest of that answer stays unread | `Voice::stop` keeps the cut conversation hushed until it asks again (`Voice::started`) | `service/voice.rs` |
| An answer arrives while the person holds or dictates | It waits for them | `speak_all` waits while `interaction::listening()` is Hold or Dictation | `service/voice.rs` |
| Dictating (any length) | Announcements wait | `interaction::Listening::Dictation`, set level by level by the listener; reasons are a set, so no capture clears another's | `interaction.rs`, `recognition.rs` |
| „Stopp“ / „Sei still“ / Escape | Quiet for a minute | Announcements wait 60 s (`service::quiet_announcements`) | `shortcut.rs` |
| Barge-in thresholds | Learn on this Mac | A false alarm raises the route's frames by 3 (and 1 dB), a take-over lowers them by a ninth of that: about one false alarm in ten (`interaction::tuning`, `hex assistant barge-in`) | `interaction/tuning.rs` |

## Jobs

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| A job runs | Frame, a card at the orb (title, step, time, live browsers), task in the list (Läuft) | Goal loop: `## Done when`, a read-only check (its guard observes only, its browsers never send, its shell is sandboxed), up to the 7-day budget | `agent::goal`, `agent::check` |
| A big job ("50 Lieferanten vergleichen, Tabelle, Team informieren") | One card with its helpers summed up; hover shows each helper under it | The job leads a team: `hex_team_start` starts helpers (Background jobs in folders inside its own, at most 12 at once, quiet, stopped with it), `hex_team_status` waits for the next to finish; the lead does what depends on their results and answers once | `agent::team`, `Runner::start_team` |
| One request whose parts build on each other ("recherchieren, Tabelle, Link per WhatsApp") | One job | `routing` keeps dependent parts in one job (its task keeps every detail and the order); unrelated requests still become separate jobs | `agent::routing`, `hex_start_jobs` |
| The person changes the plan while helpers work | The lead passes it on | `hex_team_tell` steers a helper (or all) at once, authorized only from the person's words; a steered helper stays its lead's | `agent::team`, `Runner::team_tell` |
| A helper | Does its part only | Its prompt names its lead and part, no helpers of its own (`team::helper_rules`); it stops when its lead ends | `agent::team` |
| More than three jobs | One line „N Aufträge laufen“; resting on it unfolds every card (scrolling), a press keeps them | `jobs::Layout::All`, `LineActions` | `rune_ui/assistant_overlay/jobs.rs` |
| Fifty jobs with many browsers | Only browsers a card shows stream | `live::Rate::Idle`, `live::cards` | `assistant_overlay/live.rs` |
| Memory tight | Queued jobs wait, start by themselves later | `capacity::pressure` (macOS kernel pressure level) and `background_room` | `capacity.rs` |
| Two jobs need the real mouse | One at a time | `guard::state::take_hands` (20 s wait, then BUSY) | `guard/state.rs` |
| Hex crashed with jobs running | Old agents end, jobs resume clean | `provider::orphans::sweep`, interrupted jobs' browsers closed before resuming | `provider/orphans.rs` |
| What did a job do? | Every tool call, secrets blanked; `Hex-Protokoll.txt` in its folder (listed in Chats' results) | `<assistant>/audit/<id>.jsonl`, `hex assistant audit <id>`, `audit::keep_in` | `audit.rs` |
| A job needs something | Inbox bubble „<Aufgabe> fragt …“, read when nothing else plays | Hold answers; „später“ defers; „welche Fragen sind offen?“ reads them | `service/inbox.rs` |
| "Wie weit ist der Shop?" (hold) | Spoken status | `status_reply` from plan, elapsed, gaps | `service/status.rs` |
| Hold naming a running job | Goes to that job | `context::route` steers it | `service/context.rs` |
| Hex restarts / Mac wakes | Jobs go on | `resumable_jobs` within the budget | `service.rs` startup |
| Usage limit | Task waits, retries | `retry_on_limit` across the budget | `service/run.rs` |
| Mac idle while jobs run | Mac stays awake | IOKit assertion while Background work runs | `desktop/keep_awake.rs` |
| Job finishes | One summary line (several collapse), notification only away from Hex | `voice` queue priorities; `notify::wanted` | `service/voice.rs`, `notify.rs` |

## Setup and settings

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| Something missing after an update | Compact repair flow with only the missing steps | `setup_gaps` at launch and every 5 min | `desktop/setup_gaps.rs` |
| Hold while something essential is missing | The HUD says what's missing | Guide opens | `recognition/ask.rs` gate |
| Settings changed | Takes effect for the next task | Settings are read per task (`assistant_runtime`) | `app_settings/assistant.rs` |

## Security

- Authorization (`autonomy::from_request`, the plan's `HEX_AUTHORIZED`
  check) reads only the person's own words (`service::Said`): never a
  hand-off's or a helper's task an agent wrote, nor a schedule an agent made
  (`Schedule::by_person`).
- A yes to a confirmation covers that kind of action for 15 minutes
  (`autonomy::CONFIRMED_FOR_MS`), not the rest of the job.
- A background job's Hex tools (`tools::Role::Job`) can't remember,
  schedule, unschedule, or hand off; it leads a team instead.

## Gaps left

- **Mac build unverified here:** the Linux server can't compile macOS code;
  `recognition/*` (dictation listening, conversation restart, barge-in
  tuning hooks) was reviewed by reading only.
- Take over gates desktop and browser actions only; a Claude Code job's
  shell and file tools go on (a PreToolUse hook would need Hex to stop
  disabling hooks wholesale).
- Linux and Windows dictation doesn't set `Listening::Dictation` (their
  audio loop only runs while recording; it needs a level set from the main
  loop).
- Step texts on cards ("Opening nike.com in the browser") are English.
- Barge-in tuning learns frames and the margin only; the echo judge's own
  constants (`OVER_SHARE`, `PATH_PERCENTILE`) stay fixed.
