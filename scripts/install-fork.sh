#!/bin/sh
# Builds Agent-Desk Dev from this checkout, signs it with this Mac's Apple
# Development certificate, installs it as /Applications/Agent-Desk Dev.app, and
# removes every other development copy of Agent-Desk on this Mac.
#
# Two apps, side by side (docs/pilot-release.md):
# - Live: /Applications/Agent-Desk.app (fheuer.agentdesk), the Pilot release; updates itself.
# - Dev:  /Applications/Agent-Desk Dev.app (fheuer.agentdesk.dev), this checkout; never
#   updates, its own data (~/Library/Application Support/agent-desk-dev), a DEV
#   band on its icon. Starting one quits the other: they share the
#   shortcuts and the microphone.
# Old target/app bundles, Agent-Desk Test builds, Hex Dev and Hex Test from
# before the rename, and stray downloads of Agent-Desk go
# (`agent-desk dev copies --remove`), so a permission in System Settings always
# belongs to Live or Dev. A stable signature keeps Dev's permissions across
# rebuilds.
#
#   scripts/install-fork.sh            build, install, clean up
#   scripts/install-fork.sh --open     … and open Agent-Desk Dev
#   scripts/install-fork.sh --dry-run  only list what the clean-up removes
#
# AGENT_DESK_CODESIGN_IDENTITY picks another certificate; the team comes from it.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

open_after=
for argument in "$@"; do
  case "$argument" in
    --open) open_after=1 ;;
    --dry-run)
      # The installed Dev build (or any built hex) lists the copies.
      hex="/Applications/Agent-Desk Dev.app/Contents/MacOS/agent-desk"
      [ -x "$hex" ] || hex="$root/target/release/agent-desk"
      [ -x "$hex" ] || { echo "Build Agent-Desk first (cargo build --release)." >&2; exit 1; }
      exec "$hex" dev copies --keep "/Applications/Agent-Desk Dev.app"
      ;;
    *) echo "usage: $0 [--open] [--dry-run]" >&2; exit 2 ;;
  esac
done

identity=${AGENT_DESK_CODESIGN_IDENTITY:-}
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning \
    | sed -n 's/.*"\(Apple Development:[^"]*\)".*/\1/p' | head -1)
fi
if [ -z "$identity" ]; then
  echo "No Apple Development certificate found. Create one in Xcode (Settings › Accounts › Manage Certificates)." >&2
  exit 1
fi
team_id=$(security find-certificate -c "$identity" -p \
  | openssl x509 -noout -subject -nameopt multiline \
  | sed -n 's/^ *organizationalUnitName *= *//p' | head -1)
if [ -z "$team_id" ]; then
  echo "Couldn't read the team of \"$identity\"." >&2
  exit 1
fi

echo "Signing with: $identity (team $team_id)" >&2
installed=$(AGENT_DESK_DEV_BUILD=1 \
AGENT_DESK_LOCAL_BUILD=1 \
AGENT_DESK_ALLOW_DEVELOPMENT_SIGNING=1 \
AGENT_DESK_CODESIGN_IDENTITY="$identity" \
AGENT_DESK_TEAM_ID="$team_id" \
AGENT_DESK_OPEN="$open_after" \
  "$root/scripts/install-app.sh")
echo "Installed $installed" >&2

# Every other development copy goes, so grants belong to Live or Dev only.
"$installed/Contents/MacOS/agent-desk" dev copies --remove --keep "$installed"

