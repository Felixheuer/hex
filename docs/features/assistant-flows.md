# Assistant flows, as the person meets them

How every assistant flow looks and sounds from the outside, what the code
does for it, and the gaps that remain. Principle: everything works by voice
and on the screen's overlay, without the Hex window; one gesture (hold the
Action shortcut and speak), and the assistant decides the rest.
How overlapping moments resolve is in `src/assistant/interaction.rs`.

Audited on dev @ 9c2f35ff (pilot.15) by reading the code and the
fake-backend service tests; "Gap" rows are open.

## Asking

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| Hold, short question about the window in front | HUD while held; answer bubble at the HUD, read aloud | Here task answers from the window (or the window right behind Hex when Hex itself is in front) | `recognition/ask.rs`, `computer::quick_context`, `capture::frontmost_external_app_info` |
| Hold, a job ("mach ein Werbevideo über diese Seite") | Short spoken line („Mach ich …“), frame on | **Fixed:** the agent calls `hex_start_jobs`; the runner starts the job when the run ends (`agent::handoff`). Fallbacks: the `HEX_JOB` marker, and an answer that only *claims* it started (prose) starts the whole request | `agent::handed_on`, `tools.rs` |
| Hold, several independent jobs | One line, several tasks in the list | `hex_start_jobs` with several jobs: the conversation goes on with the first job's own part, the others start as their own conversations | `service/run.rs` Escalate `first_task` |
| Hold with an app/browser request ("öffne einen Browser") | Handed on, never "do it yourself" | Here rule + ROUTING name the tool | `agent::system_prompt`, `ROUTING` |
| Hold, "Stopp" / "Halt" / "Sei still" | Hex goes quiet | Escape path; "Stopp alle Aufgaben" = Stop all | `shortcut/spoken_stop.rs` |
| Typed in Chats | Answer in the transcript | Same routing as a hold (Chat lane = Background rules, ROUTING applies) | `agent::prepare` |

## While Hex speaks

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| Hold while Hex reads aloud | Hex stops at the press | `interaction::hush` when the capture becomes real | `recognition.rs` |
| Talking over the answer (headphones) | Hex pauses; the words become the next request | barge-in 240 ms → pause; transcript decides | `conversation.rs`, `shortcut/barge_in.rs` |
| Talking over the answer (speakers) | Same, after 500 ms of voice the echo model can't explain | Empty/own-echo transcript resumes the cut sentence | `recognition/echo.rs` |
| Music or another app's audio | No hands-free window, no barge-in | Hold and "Stopp" still work | `recognition/other_audio.rs` |
| After an answer (conversation mode) | Listening ring for `conversation_seconds` | **Fixed:** the window is now a listening state of its own (`interaction::Listening::FollowUp`), so job summaries and inbox questions wait instead of being spoken into it | `interaction::listening`, `talk.rs` |
| Hex's own voice heard back | Nothing | Dropped (`speech::own_echo`) | `shortcut.rs` |

## Jobs

| Trigger | The person sees / hears | Result | Code |
|---|---|---|---|
| A job runs | Frame, task in the list (Läuft), live browser previews | Goal loop: `## Done when`, verifier, up to the 7-day budget | `agent::goal` |
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

## Gaps left (outside this change)

- **Overlay (agent B's area):** the spoken answer bubble was reported missing
  on pilot.13/14; the listening ring is placed "past the HUD" and can lie on
  top of the answer bubble. The ring should read
  `assistant::interaction::listening()` (show for `FollowUp` and `Hearing`,
  hide for `Off`; a `Hold` is shown by the HUD itself) instead of only the
  HUD's `Listening`/`Started`/`Discarded` events, and be placed so it never
  covers the answer bubble (stack it under the bubble, or tint the bubble's
  edge instead of drawing a second ring).
- **Chats (agent A's area):** status questions typed in a running job's
  conversation, duplicate request after escalation, message order.
- `asks_for_status` doesn't match „Wie weit bist du?“ (it expects a title or
  „die Aufgaben“); add second-person phrasings when a single job runs.
- Dictation (not a hold) doesn't set the listening state, so an announcement
  can still be spoken while the person dictates a long text; the hush at
  its start stops current speech only.
- Barge-in thresholds are tuned on synthetic signals; the logs now say
  `barge-in: paused|resumed|took over` to tune them on real rooms.
