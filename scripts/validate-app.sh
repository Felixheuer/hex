#!/bin/sh
set -eu

if [ "$#" -ne 4 ]; then
  echo "Usage: $0 <Agent-Desk.app path> <expected bundle name> <version> <build>" >&2
  exit 1
fi

bundle=$1
expected_name=$2
version=$3
build_number=$4
team_id=${AGENT_DESK_TEAM_ID:?Set AGENT_DESK_TEAM_ID to the Apple Developer signing team}
plist="$bundle/Contents/Info.plist"

fail() {
  echo "Rust Agent-Desk bundle validation failed: $*" >&2
  exit 1
}

plist_value() {
  /usr/libexec/PlistBuddy -c "Print :$1" "$plist"
}

[ -d "$bundle" ] || fail "$bundle does not exist"
[ "$(basename "$bundle")" = "$expected_name" ] || fail "expected $expected_name, found $(basename "$bundle")"
[ "$(plist_value CFBundleIdentifier)" = fheuer.agentdesk ] || fail "wrong bundle identifier"
[ "$(plist_value CFBundleExecutable)" = agent-desk ] || fail "wrong executable name"
[ "$(plist_value CFBundleDisplayName)" = Agent-Desk ] || fail "wrong display name"
[ "$(plist_value CFBundleName)" = Agent-Desk ] || fail "wrong bundle name"
[ "$(plist_value CFBundleShortVersionString)" = "$version" ] || fail "wrong version"
[ "$(plist_value CFBundleVersion)" = "$build_number" ] || fail "wrong build number"
[ "$(plist_value LSMinimumSystemVersion)" = 15.0 ] || fail "wrong minimum macOS"
[ "$(plist_value CFBundleURLTypes:0:CFBundleURLSchemes:0)" = agent-desk ] || fail "wrong URL scheme"
# Agent-Desk updates itself through the Anomaly release service; Sparkle is gone.
! plist_value SUFeedURL >/dev/null 2>&1 || fail "a Sparkle feed is still configured"
# 2.3.1 and Swift Agent-Desk's Sparkle installs this app only if it keeps their update key.
[ "$(plist_value SUPublicEDKey)" = mIek27lttJe8cIBqVZFhh6reRKjpTx1h9ZY9OKWPtuM= ] || fail "the legacy Sparkle key is missing"
[ ! -e "$bundle/Contents/Frameworks/Sparkle.framework" ] || fail "Sparkle.framework is still bundled"
[ "$(plist_value LSUIElement)" = true ] || fail "menu-bar application policy is disabled"
[ -x "$bundle/Contents/MacOS/agent-desk" ] || fail "packaged executable is missing"
# The `capture` experiment's movie decoder, and the camera its iPhone mirror asks for.
[ -x "$bundle/Contents/MacOS/scrawl-helper" ] || fail "scrawl-helper is missing"
plist_value NSCameraUsageDescription >/dev/null 2>&1 || fail "Info.plist does not explain the camera"
[ "$(lipo -archs "$bundle/Contents/MacOS/agent-desk")" = arm64 ] || fail "packaged executable is not arm64-only"
[ -x "$bundle/Contents/Library/HexMicrophone.driver/Contents/MacOS/HexMicrophoneDriver" ] || fail "bundled HexMicrophone.driver is missing"
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$bundle/Contents/Library/HexMicrophone.driver/Contents/Info.plist")" = fheuer.agentdesk.microphone.driver ] || fail "bundled driver has wrong bundle identifier"
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$bundle/Contents/Library/HexMicrophone.driver/Contents/Info.plist")" = "$version" ] || fail "bundled driver has wrong version"

codesign --verify --deep --strict --verbose=2 "$bundle"
signature=$(codesign -d --verbose=4 "$bundle" 2>&1)
printf '%s\n' "$signature" | grep -Fq 'Identifier=fheuer.agentdesk' || fail "code signature has the wrong identifier"
printf '%s\n' "$signature" | grep -Fq "TeamIdentifier=$team_id" || fail "code signature has the wrong team"
if [ "${AGENT_DESK_ALLOW_DEVELOPMENT_SIGNING:-}" = 1 ]; then
  # A build for this team's own Macs (scripts/pilot-release-macos.sh without
  # a Developer ID certificate).
  printf '%s\n' "$signature" | grep -Eq 'Authority=(Developer ID Application|Apple Development):' || fail "not signed by an Apple certificate"
else
  printf '%s\n' "$signature" | grep -Fq 'Authority=Developer ID Application:' || fail "not signed for Developer ID distribution"
fi
entitlements=$(codesign -d --entitlements - "$bundle" 2>/dev/null)
printf '%s\n' "$entitlements" | grep -Fq 'com.apple.security.device.audio-input' || fail "microphone entitlement is missing"
printf '%s\n' "$entitlements" | grep -Fq 'com.apple.security.automation.apple-events' || fail "automation entitlement is missing"
if printf '%s\n' "$entitlements" | grep -Fq 'com.apple.security.app-sandbox'; then
  fail "Rust application must not be sandboxed"
fi

echo "$bundle"
