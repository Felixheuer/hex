# Round 7: the ultimate desk worker

Goal (user, 2026-11-13): Agent-Desk works for 10 hours like a real person. It completes
tasks, thinks, uses different media, sub-agents and browsers, and keeps going until it's
done. Each part follows the same loop: plan → sub-agent → filtered tests → merge → release
→ next round. Progress is tracked in `docs/iteration-log.md`.

## A. Use Cua Driver fully

Today the guard passes on only the basic functions (windows, snapshot, click, type, scroll,
keys). Cua Driver 0.49.x offers about 60 tools.

- Offer directly:
  - `invoke_menu` (menu items without clicking)
  - `drag`, `hotkey`
  - `run_steps` / `run_script` (several actions in one round trip = much faster)
  - `verify_state` (check after every action)
  - `parse_visual_regions` + `zoom` (find elements in the picture, read details)
  - `set_window_frame`, `get_desktop_state`, clipboard
- Every one goes through the guard: leases, consent, parking, background first.
- The person's own browser through Cua's `browser_*` tools (extension) instead of
  JavaScript from Apple Events. The extension is installed and checked during setup.
- Record and replay: record successful procedures (`start_recording`), store them per app
  as a "skill", and replay them on the next similar task (`replay_trajectory`). Check the
  result afterwards.
- Show Cua's agent cursor in the live minis, so you can see where it clicks.
- Daily update (done) plus a capability check: new Cua tools are listed automatically.

## B. Long-running autonomy (10 hours)

- Job journal: `journal.md` in the job folder, updated automatically (what was done,
  learned, open). It is the job's memory across context limits.
- Context management: when the Claude Code session grows too large, start a fresh session
  with the journal and plan summarized, so nothing is lost and it doesn't slow down.
- Explicit success criteria per plan step. After every step check it with `verify_state`,
  files, or screenshot comparison. When it fails, switch strategy (other method / tool /
  helper), not repeat the same thing.
- Awareness of time and budget: a job knows how long it has, plans breaks and intermediate
  results, and reports only when asked (or at milestones the person asked for).
- Robustness: resume after a crash, update, sleep or lock screen from the journal; network
  outages; usage limits (wait); hanging apps (reopen).

## C. Media

- Documents: read and write PDF, Word, Excel, PowerPoint and Pages/Numbers/Keynote
  (scripted or with libraries); compare and summarize.
- Images: analyze, crop, annotate, compare (exists: `hex_compare_images`); generate images
  through an available service when the person wants it.
- Audio: transcribe files (own model), read text aloud (own voice), cut audio.
- Video: watch (exists); create videos (screen recording of the hidden browser or an app +
  ffmpeg/Remotion, music and voiceover, export).

## D. Sub-agent orchestration

- Helpers by workload (exists: teams up to 32): a shared blackboard (`team.md`) for
  findings, no double work.
- Merge the results of all helpers automatically (tables, reports).
- Show helper progress in the rail (exists), conflict-free use of browsers and windows.

## E. Learning and memory

- Procedural memory: what worked in which app (method, menu path, script) is remembered
  per app version and preferred next time.
- Remember the person's preferences (tone, recipients, folders) from corrections.

## F. Real testing

- `agent-desk bench` against Dev with test contacts, once the person frees up the Mac.
  Findings go into the next round.
- The full test suite once, on its own, when the person agrees (audio).
