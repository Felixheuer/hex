# Agent-Desk: iteration log

How it works: each round is merge → check → release (Pilot) → new todos → next round. The
goal comes from the user: everything of a piece, universal (no special cases per app), fast,
maintainable, and nothing that isn't possible.

## Round 3 (running, 2026-11-13)

- [ ] Cua Driver as the desktop backend (background like Codex, cursor and focus stay with
      the person), agent-desktop as fallback
- [x] Universal app control: `hex_app_capabilities` (sdef, URL schemes, AX), no recipe list,
      no brand word lists (pilot.40)
- [ ] Orb view: "All live" overview, scrolling, nothing clipped, checked visually with
      headless renders
- [x] Permissions: Contacts/Reminders/Calendar/Automation/FDA in setup and repair
- [x] Stable service tests (46/46)
- [x] Browsers are Chrome with live pictures (pilot.39)

## Round 4 (planned): integration and quality

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

## Round 5 (planned): rename and real test

- [ ] Rename Hex → Agent-Desk (display name, CLI, data folder with migration, bundle ID
      fheuer.agentdesk, URL scheme); the LICENSE copyright and the upstream sync stay
- [ ] Real benchmark against Dev (`hex bench`) once the Mac is free; findings → next round
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
