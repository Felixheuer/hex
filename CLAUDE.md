# Hex for Claude Code

`AGENTS.md` is the guide for this repo. It is large (about 125 KB), so grep it
for the section you need instead of reading it whole.

## Rules that protect the person's Mac

- **Never run the whole test suite more than once at a time.** Many tests
  open CoreAudio devices and listeners. Several unfiltered
  `cargo test --bin hex` runs at once (parallel subagents, worktrees, or
  terminals) once wedged `coreaudiod` and the Mac had no sound until
  `sudo killall coreaudiod`. Filter to the modules you touch, and avoid
  audio, microphone, feedback, meeting, and recording tests unless you
  changed them. Run the full suite alone, once, at the end. Subagents never
  run it.
- Don't launch the app, inject key or mouse events, or let assistant tasks
  click, type, or close apps on this Mac while testing; the person is
  using it. Read-only desktop checks only.
- This is a fork: commit and push to `Felixheuer/hex`, never to
  `anomalyco/hex`. Its default branch is `dev`; there is no `main`.
- The fork has no CI: `.github/workflows` is removed and GitHub Actions is
  turned off for the repository. Where AGENTS.md or docs mention workflows,
  CI, golden artifacts from `check-linux.yml`, or release jobs, they describe
  upstream. Verify locally instead (filtered tests, `scripts/check`).
