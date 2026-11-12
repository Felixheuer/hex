#!/bin/sh
# Builds Hex Dev from this checkout, signs it with this Mac's Apple
# Development certificate, installs it as /Applications/Hex Dev.app, and
# removes every other development copy of Hex on this Mac.
#
# Two apps, side by side (docs/pilot-release.md):
# - Live: /Applications/Hex.app (anomaly.hex), the Pilot release; updates itself.
# - Dev:  /Applications/Hex Dev.app (anomaly.hex.dev), this checkout; never
#   updates, its own data (~/Library/Application Support/hex-dev), a DEV
#   band on its icon. Starting one quits the other: they share the
#   shortcuts and the microphone.
# Old target/app bundles, Hex Test builds, and stray downloads of Hex go
# (`hex dev copies --remove`), so a permission in System Settings always
# belongs to Live or Dev. A stable signature keeps Dev's permissions across
# rebuilds.
#
#   scripts/install-fork.sh            build, install, clean up
#   scripts/install-fork.sh --open     … and open Hex Dev
#   scripts/install-fork.sh --dry-run  only list what the clean-up removes
#
# HEX_CODESIGN_IDENTITY picks another certificate; the team comes from it.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

open_after=
for argument in "$@"; do
  case "$argument" in
    --open) open_after=1 ;;
    --dry-run)
      # The installed Dev build (or any built hex) lists the copies.
      hex="/Applications/Hex Dev.app/Contents/MacOS/hex"
      [ -x "$hex" ] || hex="$root/target/release/hex"
      [ -x "$hex" ] || { echo "Build Hex first (cargo build --release)." >&2; exit 1; }
      exec "$hex" dev copies --keep "/Applications/Hex Dev.app"
      ;;
    *) echo "usage: $0 [--open] [--dry-run]" >&2; exit 2 ;;
  esac
done

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
installed=$(HEX_DEV_BUILD=1 \
HEX_LOCAL_BUILD=1 \
HEX_ALLOW_DEVELOPMENT_SIGNING=1 \
HEX_CODESIGN_IDENTITY="$identity" \
HEX_TEAM_ID="$team_id" \
HEX_OPEN="$open_after" \
  "$root/scripts/install-app.sh")
echo "Installed $installed" >&2

# Every other development copy goes, so grants belong to Live or Dev only.
"$installed/Contents/MacOS/hex" dev copies --remove --keep "$installed"

