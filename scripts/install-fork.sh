#!/bin/sh
# Builds Hex from this checkout, signs it with this Mac's Apple Development
# certificate, and installs it as /Applications/Hex.app, then opens it.
#
# A stable signature keeps macOS permissions (Accessibility, Screen
# Recording, Microphone) across rebuilds; an ad-hoc signed build loses them
# every time. The build is a local build: it shows its commit and never
# checks anomalyco's update server (scripts/update-fork.sh updates it).
#
#   scripts/install-fork.sh
#
# HEX_CODESIGN_IDENTITY picks another certificate; the team comes from it.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

identity=${HEX_CODESIGN_IDENTITY:-}
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
HEX_LOCAL_BUILD=1 \
HEX_ALLOW_DEVELOPMENT_SIGNING=1 \
HEX_CODESIGN_IDENTITY="$identity" \
HEX_TEAM_ID="$team_id" \
  "$root/scripts/install-app.sh"
