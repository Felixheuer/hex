#!/bin/sh
# Agent-Desk's self-improvement loop (docs/roadmap-agent-desk.md, I;
# docs/bench.md): install Agent-Desk Dev from this checkout, run `agent-desk bench`
# against it, and hand the findings to a Claude Code session that fixes
# them in its own worktree and opens a PR when the bench isn't worse.
#
# Starting Agent-Desk Dev QUITS Agent-Desk (Live): they share the shortcuts and the
# microphone. So this runs only when the person allows it:
#
#   scripts/self-improve.sh --when idle        only if nobody touched the Mac
#                                             for 20 minutes (HIDIdleTime)
#   scripts/self-improve.sh --when idle=45     … for 45 minutes
#   scripts/self-improve.sh --now              the person says "go" (testing)
#
# Then:
#   (default)        install Dev, bench it, start the Claude Code loop
#   --bench-only     install Dev, bench it, stop (a night run without fixes)
#   --no-install     skip scripts/install-fork.sh (Dev is current)
#   --allow-front    let the bench run tasks that bring an app forward
#                    (only sensible with --when idle)
#   --keep-dev       leave Agent-Desk Dev running afterwards (default: reopen Agent-Desk
#                    if it ran before)
#   --print-prompt   print the Claude Code prompt and exit
#   --dry-run        say what would happen; validate the bench; change nothing
#
# The rules the loop keeps (CLAUDE.md): never the whole test suite (only
# `cargo test --bin agent-desk <filter>`), never audio, microphone, meeting,
# recording, or feedback tests, never the person's front window, messages
# only as drafts to the test recipients in bench/config.toml, nothing pushed
# to anomalyco/hex, and no release.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
prompt_file="$root/bench/self-improve.md"
dev_app="/Applications/Agent-Desk Dev.app"
live_app="/Applications/Agent-Desk.app"
dev_hex="$dev_app/Contents/MacOS/agent-desk"

when=
idle_minutes=20
mode=loop
install=1
allow_front=
keep_dev=
dry_run=
for argument in "$@"; do
  case "$argument" in
    --now) when=now ;;
    --when) when=next-idle ;;
    --when=idle|idle) when=idle ;;
    --when=idle=*|idle=*) when=idle; idle_minutes=${argument##*=} ;;
    --bench-only) mode=bench ;;
    --no-install) install= ;;
    --allow-front) allow_front=--allow-front ;;
    --keep-dev) keep_dev=1 ;;
    --dry-run) dry_run=1 ;;
    --print-prompt) cat "$prompt_file"; exit 0 ;;
    -h|--help) sed -n '2,31p' "$0"; exit 0 ;;
    *) echo "usage: $0 (--now | --when idle[=MIN]) [--bench-only] [--no-install] [--allow-front] [--keep-dev] [--dry-run] [--print-prompt]" >&2; exit 2 ;;
  esac
done
# `--when idle` as two words.
[ "$when" = next-idle ] && when=idle

say() { printf 'self-improve: %s\n' "$*" >&2; }

# Seconds since the last keyboard or mouse input.
idle_seconds() {
  ioreg -c IOHIDSystem | awk '/HIDIdleTime/ { print int($NF / 1000000000); exit }'
}

case "$when" in
  now) say "the person allowed it (--now)" ;;
  idle)
    case "$idle_minutes" in
      ''|*[!0-9]*) echo "--when idle=MIN needs whole minutes" >&2; exit 2 ;;
    esac
    idle=$(idle_seconds)
    if [ "${idle:-0}" -lt $((idle_minutes * 60)) ]; then
      say "the Mac was used $((${idle:-0} / 60)) min ago (< $idle_minutes min): not now"
      exit 0
    fi
    say "idle for $((idle / 60)) min (≥ $idle_minutes min)"
    ;;
  *)
    echo "Starting Agent-Desk Dev quits Agent-Desk. Run with --now (you allow it) or --when idle[=MIN]." >&2
    exit 2
    ;;
esac

cd "$root"
[ -f bench/config.toml ] || say "no bench/config.toml: message and reading tasks are skipped (see bench/config.example.toml)"

if [ -n "$dry_run" ]; then
  say "dry run: would install Agent-Desk Dev, open it, run \`agent-desk bench run --target dev\`, then $( [ "$mode" = loop ] && echo "start Claude Code with bench/self-improve.md" || echo "stop")"
  exec cargo run --quiet --bin agent-desk -- bench run --dry-run --dir "$root/bench"
fi

live_was_running=
pgrep -f "$live_app/Contents/MacOS/" >/dev/null 2>&1 && live_was_running=1

if [ -n "$install" ]; then
  say "installing Agent-Desk Dev (scripts/install-fork.sh)"
  "$root/scripts/install-fork.sh"
fi
[ -x "$dev_hex" ] || { echo "No $dev_app; run scripts/install-fork.sh." >&2; exit 1; }

# Agent-Desk Dev must run for the bench; opening it quits Agent-Desk (allowed above).
if ! "$dev_hex" status >/dev/null 2>&1; then
  say "opening Agent-Desk Dev (this quits Agent-Desk)"
  open -g "$dev_app"
  tries=0
  until "$dev_hex" status >/dev/null 2>&1; do
    tries=$((tries + 1))
    [ "$tries" -gt 60 ] && { echo "Agent-Desk Dev didn't answer within a minute." >&2; exit 1; }
    sleep 1
  done
fi

restore() {
  if [ -n "$live_was_running" ] && [ -z "$keep_dev" ]; then
    say "reopening Agent-Desk (this quits Agent-Desk Dev)"
    open -g "$live_app" || true
  fi
}

say "running the bench against Agent-Desk Dev"
bench_status=0
"$dev_hex" bench run --target dev --dir "$root/bench" $allow_front || bench_status=$?
case "$bench_status" in
  0) say "not worse than the run before" ;;
  1) say "worse than the run before (see bench/results and the table above)" ;;
  *) say "the bench couldn't run (exit $bench_status)"; restore; exit "$bench_status" ;;
esac

if [ "$mode" = bench ]; then
  restore
  exit "$bench_status"
fi

if ! command -v claude >/dev/null 2>&1; then
  say "Claude Code isn't installed; the results are in bench/results. Start it yourself with: claude \"\$(cat bench/self-improve.md)\""
  restore
  exit "$bench_status"
fi

say "starting the Claude Code loop (bench/self-improve.md)"
AGENT_DESK_SELF_IMPROVE=1 AGENT_DESK_SELF_IMPROVE_ALLOW_FRONT=${allow_front:+1} \
  claude "$(cat "$prompt_file")" || true
restore
