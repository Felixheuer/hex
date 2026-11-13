# Agent-Desk: iteration log

How it works: each round is merge → check → release (Pilot) → new todos → next round. The
goal comes from the user: everything of a piece, universal (no special cases per app), fast,
maintainable, and nothing that isn't possible.

## Round 3 (running, 2026-11-13)

- [x] Cua Driver as the desktop backend (background like Codex, cursor and focus stay with
      the person), agent-desktop as fallback (pilot.42)
- [x] Universal app control: `hex_app_capabilities` (sdef, URL schemes, AX), no recipe list,
      no brand word lists (pilot.40)
- [x] Orb view: "All live" overview, scrolling, nothing clipped, checked visually with
      headless renders (pilot.41)
- [x] Permissions: Contacts/Reminders/Calendar/Automation/FDA in setup and repair
- [x] Stable service tests (46/46)
- [x] Browsers are Chrome with live pictures (pilot.39)

## Round 4 (planned): integration and quality

- [x] Review done: docs/review-round4.md (38 findings)
- [x] P0 1, 2, 6, 7 · P1 15, 16, 17, 18, 19, 21, 22, 23 (Cua seam), 27, 28 · P2 30, 31, 32, 33,
      34, 35, 37 (pilot.42–43)
- [x] Crash: system-wide AX reads off the main thread (pilot.44); the release opens Agent-Desk
      reliably
- [x] P0 3, 4, 5 · P1 8–14, 20, 36 (pilot.45–46)
- [x] P1 24 (assistant::view), 26 (settings names), 29 (partial: hooks), 38 (splits), flaky
      tests, inbox/voice wording (pilot.47–48)
- [ ] Remaining: cfg!(test) sites in guard/space/driver/input (each needs a hook), split
      browser.rs again (warm), check Linux/Windows builds
- [ ] P0 3 (tool allowlist per backend), 4 (parking store, windows back after a restart),
      5 (an escalated job's browsers), P1 8–14 (tools per lane, guard rules, routing, helper
      rules, speed rules for jobs, exclusive desk), 20 (consent without app lists), 36 (guard
      concurrency)
- [ ] P1 24 (one view reducer for overlay and stream), 25 (renames), 26 (setting names),
      29 (cfg!(test) as hooks), 38 (splits); remaining German branches in service/inbox.rs and
      voice.rs
- [ ] Architecture review across `src/assistant/**`, overlay and stream: duplication, seams,
      dead code (screenshots, Labs leftovers), naming, module sizes (code_shape)
- [ ] UX review of the whole flow (hold → answer/action/job → minis → question → result) in
      headless renders, in German and English
- [ ] One consistent toolset in the prompt: tool names and descriptions, no overlaps (guard,
      actions, knowledge, browser)
- [ ] Error paths: every error the person sees has a way forward (no dead ends)
- [ ] Docs: AGENTS.md / docs/features/assistant.md brought up to date
- [ ] Consent checks without app lists: `guard/proxy/consent.rs` (Return sends in chat apps),
      `browser/consent.rs` (mail hosts), `knowledge/graph.rs` (messenger names), recognized by
      element/role/label instead of name
- [ ] Timing-flaky test: `a_paused_task_waits_and_then_is_told_to_look_again`

## Round 7: the ultimate desk worker (docs/roadmap-round7.md)

- [x] Background stays background (Chrome tab ids, no front on open, only used browsers as
      minis) (pilot.59)
- [x] B: 10-hour jobs: journal, fresh sessions, checks, budget, resume (pilot.60)
- [x] Window + onboarding redesign (8-step setup) (pilot.61)
- [x] C: media tools (pilot.62)
- [x] A: Cua fully used (batches, scripts, menus, verify, skills) (pilot.63)
- [ ] D+E: team blackboard, learning (running)
- [ ] Projects with soul, folder, picker (running)
- [ ] Compact the rules (Here ~11.2k, jobs ~18.8k characters), menu-bar menu, first-run voice
      screens into the new flow
- [ ] F: real bench against Dev + full suite once (with the user's OK)
- [x] OpenCode 1.x and 2 both run tasks (classic had failed on `--standalone`); DeepSeek by
      own key end to end; Flash for quick turns with hand-over to V4 Pro; "no credit left"
- [x] Google/YouTube no longer copied from Chrome into hidden browsers (signed the person
      out of Google in Chrome when agents started); old copies cleaned once
- [x] Job rules back within budget
- [x] OpenCode: watchdog fits its whole-part output (120 s), cost per job in the journal,
      a deleted conversation's OpenCode sessions deleted too; teams, checks and workspaces
      verified end to end with DeepSeek
- [x] OpenCode quick turns over ACP: started ahead, streamed, kept for the follow-up
      (first token 1.2–2.2 s instead of 4.5–5 s with DeepSeek Flash)
- [x] Browser audit: screenshots can't overwrite files outside Agent-Desk's folders,
      forgetting logins sticks, consent for Return/addresses in the person's Chrome
- [ ] Safari tab ids are window id + position: an action can land on a moved tab
- [ ] Other sites that bind sessions to a device (Microsoft, Meta, X) may react like
      Google to copied cookies; an allow list for the Chrome import would be safer
- [ ] Linux-only UI tests and goldens drifted from the fork's window (18 failures on Linux;
      the fork ships macOS only)

## Round 6 (after the rename): Cua only, no legacy

- [x] Remove agent-desktop completely (driver fallback, session/cursor CLI, guide in the
      prompt, install/check, onboarding strings, `assistant.desktop_driver`); Cua Driver is
      the only driver (macOS only; elsewhere tasks answer without apps)
- [x] Cua Driver updates (`driver::update`): daily check for new releases (GitHub
      trycua/cua), checksum (SHA256SUMS, checksums.txt, GitHub digest) and signature
      (team of the installed driver) and notarization verified, installed beside the
      current one, self-tested, switched when no task acts, rolled back on failure; status
      in setup/repair, `agent-desk assistant driver [status|update|rollback]`
- [ ] Overlay leftovers of agent-desktop (cursor image, failure text) go with the UX
      redesign (`rune_ui/assistant_overlay`)
- [ ] No backwards compatibility: no aliases (CLI, URL scheme, env, settings keys, tool
      names); only dated one-time migrations for data, settings, Keychain and the app bundle
- [x] UX D: the stage, the window recedes (pilot.53)
- [x] UX A: shared look, task colours, rail, live minis (pilot.54)
- [x] UX B: island (black, from the notch), live words, visible reasoning (pilot.56)
- [x] UX C: three frames, foreground announced and handed back, mini rings, rail flip rule
      (pilot.55)
- [x] UX integration: nothing covers labels, island at the notch, borrow announced first,
      ≤ 5 cards, German wording (pilot.58)
- [x] hex_watch_video and real Chrome codecs in the hidden browser (pilot.50)
- [x] Agent-Desk opens again after quit/update (orphaned guard), update target name valid
      (pilot.51), live minis 5–6 fps (pilot.52)

## Round 5 (planned): rename and real test

- [ ] Rename Agent-Desk → Agent-Desk (display name, CLI, data folder with migration, bundle ID
      fheuer.agentdesk, URL scheme); the LICENSE copyright and the upstream sync stay
- [ ] Real benchmark against Dev (`agent-desk bench`) once the Mac is free; findings → next round
- [ ] Full test suite once, on its own (CLAUDE.md)

## Releases

| Version | Content |
|---|---|
| pilot.32 | Overlay-first, agent knows the desktop, warm browsers |
| pilot.33 | Own browser in quick turns, invisible app-name marks, other desktops |
| pilot.34 | Desktop app without Labs |
| pilot.35 | Fix crash on own AX reads |
| pilot.36 | Speech bubble at the top again (notch indicator off) |
| pilot.37 | Knowledge, speed (two models, warm), bench |
| pilot.38 | Background apps, desk, recipes/OCR |
| pilot.39 | Browsers with live pictures (Chrome) |
| pilot.40 | Universal app control, permissions in setup, stable tests |
| pilot.41 | Overlay: All live, big pictures, scrolling |
| pilot.42 | Cua Driver, Codex-style background input |
| pilot.43 | Round 4: security, brain structure, errors/wording, any browser |
| pilot.44 | Fix crash (system-wide AX), reliable restart after an update |
| pilot.45 | Parking store, browser cleanup, exclusive desk, consent without app lists, parallel reads |
| pilot.46 | One toolset per lane and backend |
| pilot.47 | Clear setting names, file splits, stable tests |
| pilot.48 | One view model for overlay and stream |
| pilot.49 | Rename Hex → Agent-Desk |
| pilot.50 | Watching videos, hidden browsers with codecs, clear tab errors |
| pilot.51 | Opens again after quit/update; valid update index |
| pilot.52 | Live minis 5–6 fps |
| pilot.53 | The stage; the window only opens when asked |
| pilot.54 | Cua Driver only with verified updates; rail and shared look |
| pilot.55 | Frames with one meaning, foreground announced |
| pilot.56 | Island: black, live words, visible reasoning |
| pilot.57 | (built without the integration pass: the GitHub PR timed out) |
| pilot.58 | UX integration pass |
| pilot.59 | Background stays background, only used browsers as minis |
| pilot.60 | 10-hour jobs |
| pilot.61 | Window + onboarding redesign |
| pilot.62 | Media tools |
| pilot.63 | Cua fully used, skills |
