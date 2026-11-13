#!/bin/sh
# Builds `Agent-Desk Playground.app`: the Scrawl capture capsule in a window of its
# own (`agent-desk capsule-playground`), driven by a fake host with live tuning.
# It touches no Agent-Desk data, settings, or captures, and records and sends
# nothing, so it runs beside the installed Agent-Desk. Ad-hoc signed, for this Mac.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo build --locked --release --bin agent-desk --manifest-path "$root/Cargo.toml"
target=${CARGO_TARGET_DIR:-$root/target}
bundle="${AGENT_DESK_PLAYGROUND_OUT:-$root/target/app}/Agent-Desk Playground.app"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$target/release/agent-desk" "$bundle/Contents/MacOS/agent-desk"
cat > "$bundle/Contents/MacOS/playground" <<'SH'
#!/bin/sh
exec "$(dirname "$0")/hex" capsule-playground
SH
chmod +x "$bundle/Contents/MacOS/playground"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>playground</string>
  <key>CFBundleIdentifier</key><string>fheuer.agentdesk.playground</string>
  <key>CFBundleName</key><string>Agent-Desk Playground</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$bundle/Contents/MacOS/agent-desk"
codesign --force --sign - "$bundle"
echo "$bundle"
