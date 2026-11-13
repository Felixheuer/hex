#!/bin/bash
# Builds, signs, and publishes Pilot's Mac release from the owner's Mac
# (docs/pilot-release.md): Agent-Desk.app (bundle id fheuer.agentdesk, as installed)
# signed with this Mac's certificate, the update ZIP the app installs itself,
# and Pilot.dmg for the website. All go to ai.sms-pilot.de with rsync, and
# the server's pilot-publish.sh signs them with the release key and adds them
# to the release.
#
# One-time migration, remove after 2026-12: a Mac that still runs Hex
# (/Applications/Hex.app, anomaly.hex) only takes an update with Hex's
# identity. So the release also carries the bridge, a Hex.app with that
# identity holding Agent-Desk.app in Contents/Library, as the `macos-arm64`
# target older installs poll; Agent-Desk itself takes
# `macos-arm64-agentdesk`. Launched, the bridge installs Agent-Desk.app
# beside itself and opens it, and Agent-Desk removes the bridge
# (src/updater/macos.rs).
#
#   scripts/pilot-release-macos.sh --version X.Y.Z-pilot.N [--channel stable]
#     [--notes <file>] [--host t3@ai.sms-pilot.de] [--install] [--no-upload]
#
# Use the version the Linux build of this release has (the server's
# `pilot-publish.sh status` shows it), or a new one for a Mac-only release.
#
# Signing: a "Developer ID Application" certificate, with AGENT_DESK_NOTARY_PROFILE
# (an `xcrun notarytool store-credentials` profile), makes a notarized DMG
# anyone can open. Without one, the "Apple Development" certificate
# scripts/install-fork.sh uses signs a build for Macs of this team only
# (others see a Gatekeeper warning). Either way the app stamps its team
# (AGENT_DESK_UPDATE_TEAM_ID), and only updates signed by that team install. Keep
# signing with the same team: macOS permissions follow it.
#
# --install also puts the built app into /Applications/Agent-Desk.app
# (replacing a source build from install-fork.sh, and moving a Live
# /Applications/Hex.app out of the way): from then on it updates itself.
# AGENT_DESK_CODESIGN_IDENTITY picks the certificate.
set -euo pipefail

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
. "$root/scripts/macos-signing.sh"
version=
channel=stable
notes=
host=${PILOT_HOST:-t3@ai.sms-pilot.de}
publish=${PILOT_SERVER_PUBLISH:-/srv/pilot/bin/pilot-publish.sh}
install=
upload=1
while [ $# -gt 0 ]; do
  case "$1" in
    --version) version=$2; shift 2 ;;
    --channel) channel=$2; shift 2 ;;
    --notes) notes=$2; shift 2 ;;
    --host) host=$2; shift 2 ;;
    --install) install=1; shift ;;
    --no-upload) upload=; shift ;;
    *)
      sed -n '2,/^set -euo/p' "$0" | sed -e '/^set -euo/d' -e 's/^# \{0,1\}//' >&2
      exit 1
      ;;
  esac
done
[ -n "$version" ] || { echo "Pass --version (the release's version, e.g. 3.4.0-pilot.1)." >&2; exit 1; }
[ "$(uname -s) $(uname -m)" = "Darwin arm64" ] || { echo "Run this on an Apple silicon Mac." >&2; exit 1; }
if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before releasing." >&2
  exit 1
fi

# The certificate: Developer ID first, else Apple Development.
identity=${AGENT_DESK_CODESIGN_IDENTITY:-}
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:[^"]*\)".*/\1/p' | head -1)
fi
if [ -z "$identity" ]; then
  identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development:[^"]*\)".*/\1/p' | head -1)
fi
[ -n "$identity" ] || { echo "No Developer ID or Apple Development certificate (Xcode › Settings › Accounts)." >&2; exit 1; }
team_id=$(security find-certificate -c "$identity" -p | openssl x509 -noout -subject -nameopt multiline \
  | sed -n 's/^ *organizationalUnitName *= *//p' | head -1)
[ -n "$team_id" ] || { echo "Couldn't read the team of \"$identity\"." >&2; exit 1; }
notarize=
case "$identity" in
  "Developer ID Application:"*)
    if [ -n "${AGENT_DESK_NOTARY_PROFILE:-}${AGENT_DESK_NOTARY_KEY:-}" ]; then notarize=1; fi
    ;;
  *) export AGENT_DESK_ALLOW_DEVELOPMENT_SIGNING=1 ;;
esac
echo "Pilot $version (macOS, $channel) signed by $identity (team $team_id)${notarize:+, notarized}" >&2

restore() { git -C "$root" checkout -- Cargo.toml Cargo.lock 2> /dev/null || true; }
trap restore EXIT
"$root/scripts/set-crate-version.sh" "$root" "$version"
build_number=$(hex_build_number "$version")
export AGENT_DESK_VERSION="$version" AGENT_DESK_BUILD_NUMBER="$build_number" AGENT_DESK_BUILD_CHANNEL="$channel" \
  AGENT_DESK_UPDATE_TEAM_ID="$team_id" AGENT_DESK_TEAM_ID="$team_id" AGENT_DESK_CODESIGN_IDENTITY="$identity"
bundle=$("$root/scripts/build-app.sh")
"$root/scripts/validate-app.sh" "$bundle" Agent-Desk.app "$version" "$build_number" > /dev/null
restore

dist="$root/dist/pilot-$version"
rm -rf "$dist"
mkdir -p "$dist"
dmg="$dist/agent-desk-$version-macos-arm64.dmg"
zip="$dist/agent-desk-$version-macos-arm64.zip"
bridge_zip="$dist/hex-$version-macos-arm64.zip"
if [ -n "$notarize" ]; then
  "$root/scripts/notarize-app.sh" "$bundle" "$dmg" Pilot
else
  staging=$(mktemp -d "${TMPDIR:-/tmp}/pilot-dmg.XXXXXX")
  /usr/bin/ditto "$bundle" "$staging/Agent-Desk.app"
  ln -s /Applications "$staging/Applications"
  cp "$root/packaging/macos/dmg/background.tiff" "$staging/.background.tiff"
  cp "$root/packaging/macos/dmg/DS_Store" "$staging/.DS_Store"
  hdiutil create -quiet -fs APFS -format ULFO -volname Pilot -srcfolder "$staging" "$dmg"
  rm -rf "$staging"
  codesign --force --timestamp=none --sign "$identity" "$dmg"
fi
# The updater unpacks this beside the installed bundle and swaps it in.
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bundle" "$zip"

# One-time migration, remove after 2026-12: the bridge for installs that are
# still Hex. Its executable is Agent-Desk's (it hands off before anything
# else runs); Hex's updater checks its identity, version, and team.
bridge_root=$(mktemp -d "${TMPDIR:-/tmp}/pilot-bridge.XXXXXX")
bridge="$bridge_root/Hex.app"
mkdir -p "$bridge/Contents/MacOS" "$bridge/Contents/Library" "$bridge/Contents/Resources"
cp "$bundle/Contents/MacOS/agent-desk" "$bridge/Contents/MacOS/hex"
/usr/bin/ditto "$bundle" "$bridge/Contents/Library/Agent-Desk.app"
cp "$bundle/Contents/Resources/AppIcon.icns" "$bridge/Contents/Resources/" 2> /dev/null || true
cat > "$bridge/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key><string>Hex</string>
  <key>CFBundleName</key><string>Hex</string>
  <key>CFBundleExecutable</key><string>hex</string>
  <key>CFBundleIdentifier</key><string>anomaly.hex</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$build_number</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST
if [ -n "${AGENT_DESK_ALLOW_DEVELOPMENT_SIGNING:-}" ]; then
  codesign --force --timestamp=none --options runtime --sign "$identity" "$bridge"
else
  codesign --force --timestamp --options runtime --sign "$identity" "$bridge"
fi
codesign --verify --deep --strict "$bridge"
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bridge" "$bridge_zip"
rm -rf "$bridge_root"
ls -l "$dist" >&2

if [ -n "$upload" ]; then
  incoming="/srv/pilot/incoming/macos-$version-$$"
  ssh "$host" "mkdir -p '$incoming'"
  rsync -a --progress "$dmg" "$zip" "$bridge_zip" ${notes:+"$notes"} "$host:$incoming/"
  remote_notes=
  [ -z "$notes" ] || remote_notes="--notes '$incoming/$(basename "$notes")'"
  ssh "$host" "'$publish' publish --channel '$channel' $remote_notes '$version' '$incoming/$(basename "$zip")' '$incoming/$(basename "$bridge_zip")' '$incoming/$(basename "$dmg")' && rm -rf '$incoming'"
fi

if [ -n "$install" ]; then
  installed=/Applications/Agent-Desk.app
  if [ -d "$installed" ]; then
    existing=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$installed/Contents/Info.plist" 2> /dev/null || true)
    case "$existing" in
      fheuer.agentdesk) ;;
      *) echo "Refusing to replace $installed ($existing)." >&2; exit 1 ;;
    esac
  fi
  staged="/Applications/.Agent-Desk.app.installing"
  rm -rf "$staged"
  /usr/bin/ditto "$bundle" "$staged"
  if pgrep -f "$installed/Contents/MacOS/agent-desk" > /dev/null; then
    pkill -TERM -f "$installed/Contents/MacOS/agent-desk" || true
    for _ in $(seq 30); do pgrep -f "$installed/Contents/MacOS/agent-desk" > /dev/null || break; sleep 1; done
  fi
  rm -rf "$installed"
  mv "$staged" "$installed"
  # One-time migration, remove after 2026-12: Live was /Applications/Hex.app
  # (anomaly.hex). Its data folder moves on Agent-Desk's first launch.
  hex_app=/Applications/Hex.app
  if [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$hex_app/Contents/Info.plist" 2> /dev/null || true)" = anomaly.hex ]; then
    if pgrep -f "$hex_app/Contents/MacOS/hex" > /dev/null; then
      pkill -TERM -f "$hex_app/Contents/MacOS/hex" || true
      for _ in $(seq 30); do pgrep -f "$hex_app/Contents/MacOS/hex" > /dev/null || break; sleep 1; done
    fi
    rm -rf "$hex_app"
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$hex_app" 2> /dev/null || true
    echo "Removed $hex_app (Hex); Agent-Desk takes over its data at launch." >&2
  fi
  # LaunchServices may not know the new bundle yet (-600): try a few times,
  # and never let a failed open stop the cleanup below.
  for _ in 1 2 3 4 5; do
    "$installed/Contents/MacOS/agent-desk" start > /dev/null 2>&1 && break
    sleep 2
  done
  echo "Installed $installed: it now updates itself from ai.sms-pilot.de." >&2
fi
# The built bundle in target/app is a second copy of Agent-Desk with the same
# bundle id: the zip and DMG carry it, so it goes, and LaunchServices
# forgets it (the permission doctor would otherwise report it).
rm -rf "$bundle"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$bundle" 2> /dev/null || true
echo "Pilot $version for macOS: $dmg" >&2
