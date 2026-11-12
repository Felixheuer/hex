#!/bin/sh
# Updates this Mac's Hex from the fork: pulls `dev` from origin
# (Felixheuer/hex), then rebuilds, signs, and reinstalls it with
# scripts/install-fork.sh. Refuses to run over uncommitted changes.
#
#   scripts/update-fork.sh           update if origin/dev has new commits
#   scripts/update-fork.sh --force   rebuild and reinstall even if not
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "Uncommitted changes in $root; commit or stash them first." >&2
  exit 1
fi
branch=$(git branch --show-current)
if [ "$branch" != dev ]; then
  echo "On branch $branch; switch to dev first (git switch dev)." >&2
  exit 1
fi

git fetch --quiet origin dev
behind=$(git rev-list --count HEAD..origin/dev)
if [ "$behind" = 0 ] && [ "${1:-}" != --force ]; then
  echo "Hex is up to date with origin/dev ($(git rev-parse --short HEAD))."
  exit 0
fi
if [ "$behind" != 0 ]; then
  echo "Pulling $behind new commit(s) from origin/dev…"
  git log --oneline HEAD..origin/dev | head -20
  git merge --ff-only --quiet origin/dev
fi
exec "$root/scripts/install-fork.sh"
