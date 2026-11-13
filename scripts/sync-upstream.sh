#!/bin/sh
# Brings anomalyco/hex's `dev` (the `upstream` remote) into the fork on a
# branch of its own, so upstream fixes arrive without touching `dev` until
# they build and pass.
#
#   scripts/sync-upstream.sh          merge upstream/dev into sync/upstream-<date>,
#                                     build, and run the assistant's and the
#                                     code-shape tests
#   scripts/sync-upstream.sh --merge  also fast-forward dev to it and push
#
# On a merge conflict it stops on the branch and lists the files; resolve,
# commit, and run it again with --merge. Never pushes to upstream.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "Uncommitted changes in $root; commit or stash them first." >&2
  exit 1
fi
git remote get-url upstream >/dev/null 2>&1 \
  || git remote add upstream https://github.com/anomalyco/hex.git
git fetch --quiet upstream dev
git fetch --quiet origin dev

new=$(git rev-list --count origin/dev..upstream/dev)
if [ "$new" = 0 ]; then
  echo "The fork already has everything from upstream/dev."
  exit 0
fi
echo "$new upstream commit(s) to bring in:"
git log --oneline origin/dev..upstream/dev | head -30

sync=sync/upstream-$(date +%Y%m%d)
if git rev-parse --verify --quiet "$sync" >/dev/null; then
  git switch --quiet "$sync"
else
  git switch --quiet -c "$sync" origin/dev
fi
if ! git merge --no-edit upstream/dev; then
  echo >&2
  echo "Merge conflicts on $sync:" >&2
  git diff --name-only --diff-filter=U >&2
  echo "Resolve them, commit, then run: scripts/sync-upstream.sh --merge" >&2
  exit 1
fi

cargo build --locked
# Narrow filters only: the whole suite at once can wedge coreaudiod (CLAUDE.md).
cargo test --locked --bin agent-desk -- assistant recognition::ask shortcut_conflicts code_shape

if [ "${1:-}" = --merge ]; then
  git switch --quiet dev
  git merge --ff-only --quiet "$sync"
  git push --quiet origin dev
  echo "dev now includes upstream/dev and is pushed. Run scripts/install-fork.sh to install it."
else
  echo "Built and tested on $sync. Run scripts/sync-upstream.sh --merge to move dev there and push."
fi
