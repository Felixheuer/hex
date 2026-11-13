#!/bin/sh
# Builds Pilot's Linux release on this server (ai.sms-pilot.de) and publishes
# it to /srv/pilot with scripts/pilot-publish.sh, then redeploys the website
# (docs/pilot-release.md).
#
#   scripts/pilot-release-linux.sh [--version X.Y.Z-pilot.N] [--channel stable]
#                                  [--notes <file>] [--no-site]
#
# Without --version it takes the next Pilot build of the checked-out crate
# version: <Cargo version>-pilot.<N+1>, N being the highest already published.
# Use the same version for the Mac and Windows builds of this release
# (scripts/pilot-release-macos.sh, scripts/pilot-release-windows.ps1); each
# adds its files to it.
#
# The build is native (this host's glibc, Ubuntu 24.04: 2.39), with portable
# CPU kernels (scripts/build-linux.sh). The .deb and .rpm are built only when
# nfpm is installed. Requires a clean, committed checkout; Cargo.toml and
# Cargo.lock get the version stamp for the build and are restored afterwards.
set -eu
umask 022

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
pilot_root=${PILOT_ROOT:-/srv/pilot}
version=
channel=stable
notes=
site=1
while [ $# -gt 0 ]; do
  case "$1" in
    --version) version=$2; shift 2 ;;
    --channel) channel=$2; shift 2 ;;
    --notes) notes=$(realpath "$2"); shift 2 ;;
    --no-site) site=; shift ;;
    *)
      sed -n '2,/^set -eu/p' "$0" | sed -e '/^set -eu/d' -e 's/^# \{0,1\}//' >&2
      exit 1
      ;;
  esac
done

if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before releasing." >&2
  exit 1
fi
crate=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)
if [ -z "$version" ]; then
  last=$(jq -r --arg prefix "$crate-pilot." \
    '[.versions[].version | select(startswith($prefix)) | ltrimstr($prefix) | tonumber] | max // 0' \
    "$pilot_root/release/hex/history.json" 2>/dev/null || echo 0)
  version="$crate-pilot.$((last + 1))"
fi
echo "Pilot $version (Linux, $channel channel) from $(git -C "$root" rev-parse --short HEAD)" >&2

mkdir -p "$pilot_root/incoming"
dist=$(mktemp -d "$pilot_root/incoming/linux-$version.XXXXXX")
restore() {
  git -C "$root" checkout -- Cargo.toml Cargo.lock 2>/dev/null || true
}
trap 'restore; rm -rf "$dist"' EXIT
trap 'exit 1' HUP INT TERM

# AGENT_DESK_BUILD_CHANNEL names the channel this build follows; the updater checks
# it once the install has a receipt (install-linux.sh writes one).
executable=$(AGENT_DESK_VERSION="$version" AGENT_DESK_BUILD_CHANNEL="$channel" AGENT_DESK_RELEASE_DIST="$dist" \
  "$root/scripts/build-linux.sh")
files="$executable"
case "$(uname -m)" in
  x86_64) label=x64 ;;
  aarch64) label=arm64 ;;
esac
if command -v "${NFPM:-nfpm}" > /dev/null 2>&1; then
  for package in $("$root/scripts/package-linux.sh" "$executable" "$version" "$label" "$dist"); do
    files="$files $package"
  done
else
  echo "nfpm is not installed: publishing without the .deb and .rpm." >&2
fi
restore

set -- publish --channel "$channel"
[ -z "$notes" ] || set -- "$@" --notes "$notes"
# shellcheck disable=SC2086
PILOT_ROOT="$pilot_root" "$root/scripts/pilot-publish.sh" "$@" "$version" $files
if [ -n "$site" ]; then
  PILOT_ROOT="$pilot_root" "$root/scripts/pilot-site-deploy.sh"
fi
echo "Published Pilot $version for Linux." >&2
