# Agent-Desk self-improvement round

You are a Claude Code session in `~/dev/desktop/ai-desktop` (the Pilot fork of
Agent-Desk, becoming Agent-Desk). `scripts/self-improve.sh` started you after it
installed Agent-Desk Dev and ran the benchmark against it. Your job this round:
turn the bench's failures into findings, fix what you can, prove the fix
with the bench, and open a PR when the bench is not worse.

## Rules (CLAUDE.md, non-negotiable)

- **Never run the whole test suite.** Only `cargo test --bin agent-desk <module
  filter>` for the modules you touch. Never audio, microphone, feedback,
  meeting, or recording tests: parallel CoreAudio tests once wedged
  `coreaudiod` and the Mac lost its sound. No subagent runs tests.
- **Starting Agent-Desk Dev quits Agent-Desk (Live)**: they share the shortcuts and the
  microphone. Only start or restart Dev when `AGENT_DESK_SELF_IMPROVE=1` is set (the
  script checked the person allowed it: `--now`, or the Mac idle long enough).
  Never install a source build over `/Applications/Agent-Desk.app`.
- **Hands off the person's Mac.** No injected keys or clicks, no
  `--allow-front` unless `AGENT_DESK_SELF_IMPROVE_ALLOW_FRONT=1`. Read-only desktop
  checks only (`agent-desktop list-apps`, `list-windows`, `lsappinfo`).
- **Messages are drafts**, only to the test recipients in
  `bench/config.toml`. Never send anything to anyone.
- **Files** only under `~/Documents/Agent-Desk/bench` (the bench's temp folders)
  and your worktree.
- Commit and push to `Felixheuer/hex` only (never `anomalyco/hex`); its
  default branch is `dev`. No release to Live: that waits for the person.
- `AGENTS.md` is the repo guide; grep it for the section you need.

## Steps

1. **Read the run.** `agent-desk bench report` (or `"/Applications/Agent-Desk Dev.app/Contents/MacOS/agent-desk" bench report`)
   shows the newest result in `bench/results/` against the one before.
   For each failed, timed-out, or errored task read its conversation and
   tool calls: `agent-desk assistant show <conversation>` and `agent-desk assistant audit
   <conversation>` from the Dev app's `agent-desk` (the result JSON has the
   conversation id), and Dev's log (`agent-desk logs`).
2. **Write findings** into `bench/findings.md` (create it if missing), newest round on top, one
   entry per failure:
   - task id and language, expectation (its checks), what happened
     (outcome, answer, the failing check's detail), timings;
   - the cause you found (file and function), or "unknown" with what you
     ruled out;
   - whether it is the app's fault or the task's (an unfair check is fixed
     in `bench/tasks/`, and says so).
3. **Fix in a worktree.** `git worktree add ../ai-desktop-improve-<date> -b
   self-improve/<date> origin/dev`, and work only there. Small, focused
   changes; a unit test for each fix where the module has tests. Verify with
   `cargo check --bin agent-desk` and filtered tests only
   (`cargo test --bin agent-desk bench::`, `cargo test --bin agent-desk assistant::guard`,
   …).
4. **Rebuild Dev and rerun.** From the worktree: `scripts/install-fork.sh`,
   then open Agent-Desk Dev (`open -g "/Applications/Agent-Desk Dev.app"`, only with
   `AGENT_DESK_SELF_IMPROVE=1`), wait until `agent-desk status` answers, and run
   `"/Applications/Agent-Desk Dev.app/Contents/MacOS/agent-desk" bench run --target dev --dir bench`
   (add `--filter` for the tasks you fixed first, then the whole bench once).
   Exit 0 means not worse than the run before; 1 means worse.
5. **Open a PR when not worse.** Commit the fix, the new
   `bench/results/*.json`, and `bench/findings.md` on your branch; push to
   `origin` (Felixheuer/hex) and open a PR against `dev` whose body has the
   comparison table (`agent-desk bench report`), what was found, fixed, and
   measured. If it is worse, don't open a PR: write why into the findings
   and stop.
6. **Changelog.** End `bench/findings.md`'s round with three lines: found,
   fixed, measured (pass count and median first-answer and total time, before
   → after).

Stop after one round, or earlier when nothing failed. Leave Agent-Desk Dev running;
the script reopens Agent-Desk if it ran before.
