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
- Two apps on this Mac, side by side: **Live** is `/Applications/Hex.app`
  (`anomaly.hex`), the published Pilot release that updates itself; **Dev**
  is `/Applications/Hex Dev.app` (`anomaly.hex.dev`, `HEX_DEV_BUILD=1`),
  this checkout, with its own data (`hex-dev`), URL scheme (`hex-dev://`),
  and a DEV band on its icon. Starting one quits the other (shared
  shortcuts and microphone). `scripts/install-fork.sh` builds, signs with
  the Apple Development certificate (so Dev's permissions survive rebuilds),
  installs Hex Dev without opening it (`--open` opens it), and removes every
  other copy of Hex (old `target/app` bundles, Hex Test, stray downloads;
  `--dry-run` lists them; `hex dev copies`). Never install a source build
  over Live. `scripts/update-fork.sh` pulls `origin/dev` and reinstalls Dev;
  `scripts/sync-upstream.sh [--merge]` brings in anomalyco's `dev` on a
  `sync/upstream-<date>` branch first.
- The fork has no CI: `.github/workflows` is removed and GitHub Actions is
  turned off for the repository. Where AGENTS.md or docs mention workflows,
  CI, golden artifacts from `check-linux.yml`, or release jobs, they describe
  upstream. Verify locally instead (filtered tests, `scripts/check`).
- The assistant is on by default in this fork (`assistant.enabled`, not a
  Lab; see `docs/features/assistant.md`), overriding upstream's "new
  features stay in Labs, off" rule for it.
- Releases of this fork are **Pilot**, self-hosted at `https://ai.sms-pilot.de`
  (website, downloads, update channel): `docs/pilot-release.md`. The updater's
  `ORIGIN` is `https://ai.sms-pilot.de/release`; every update target must carry
  an ed25519 signature by the fork's release key (`src/updater/signature.rs`;
  the private key is only on the server, `~/.config/pilot-release/`). Never
  commit a private key. Source builds (`install-fork.sh`) never check for
  updates.
