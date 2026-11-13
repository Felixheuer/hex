#!/bin/sh

# Sourced by the app and helper packagers. Print only the selected identity so
# callers can capture it without mixing diagnostics into codesign arguments.
hex_codesign_identity() {
  team_id=$1
  identity=${AGENT_DESK_CODESIGN_IDENTITY:-}
  if [ -z "$identity" ]; then
    identity=$(security find-identity -v -p codesigning | sed -n "s/.*\"\(Developer ID Application:.*($team_id)\)\"/\1/p" | head -1)
  fi
  if [ -z "$identity" ]; then
    echo "No Developer ID signing identity found for team $team_id. Set AGENT_DESK_CODESIGN_IDENTITY." >&2
    return 1
  fi
  case "$identity" in
    "Developer ID Application:"*"($team_id)") ;;
    # A build for the signer's own Mac (scripts/install-fork.sh): an Apple
    # Development certificate keeps macOS permissions across rebuilds, but
    # can't be notarized or shared.
    "Apple Development:"*)
      if [ "${AGENT_DESK_ALLOW_DEVELOPMENT_SIGNING:-}" != 1 ]; then
        echo "Signing identity is an Apple Development identity; set AGENT_DESK_ALLOW_DEVELOPMENT_SIGNING=1 for a build for this Mac only." >&2
        return 1
      fi
      ;;
    *)
      echo "Signing identity is not a Developer ID Application identity for team $team_id." >&2
      return 1
      ;;
  esac
  printf '%s\n' "$identity"
}

# Submit one artifact and wait. A local notarytool keychain profile wins;
# otherwise CI authenticates with an App Store Connect API key file.
hex_notarize() {
  if [ -n "${AGENT_DESK_NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$1" --keychain-profile "$AGENT_DESK_NOTARY_PROFILE" --wait
  else
    xcrun notarytool submit "$1" \
      --key "${AGENT_DESK_NOTARY_KEY:?Set AGENT_DESK_NOTARY_PROFILE, or AGENT_DESK_NOTARY_KEY with AGENT_DESK_NOTARY_KEY_ID and AGENT_DESK_NOTARY_ISSUER}" \
      --key-id "${AGENT_DESK_NOTARY_KEY_ID:?Set AGENT_DESK_NOTARY_KEY_ID to the App Store Connect API key ID}" \
      --issuer "${AGENT_DESK_NOTARY_ISSUER:?Set AGENT_DESK_NOTARY_ISSUER to the App Store Connect issuer ID}" \
      --wait
  fi
}

hex_version() {
  printf '%s\n' "${AGENT_DESK_VERSION:-$(cargo metadata --no-deps --format-version 1 --manifest-path "$1/Cargo.toml" | jq -r '.packages[] | select(.name == "hex") | .version')}"
}

# CFBundleVersion: the run number of a dev build (0.0.0-dev-N), else from
# X.Y.Z (a prerelease shares its release's number). Agent-Desk's updater compares
# neither; it follows the channel's current release.
hex_build_number() {
  if [ -n "${AGENT_DESK_BUILD_NUMBER:-}" ]; then
    printf '%s\n' "$AGENT_DESK_BUILD_NUMBER"
    return
  fi
  case "$1" in
    0.0.0-dev-[0-9]*) printf '%s\n' "${1#0.0.0-dev-}" ;;
    *) printf '%s\n' "${1%%-*}" | awk -F. '{ print ($1 * 10000) + ($2 * 100) + $3 }' ;;
  esac
}
