#!/usr/bin/env bash
# sign-linux.sh <dist>: signs each Linux executable in <dist>
# (hex-<version>-linux-<x64|arm64>) with Anomaly's ed25519 release key,
# AGENT_DESK_RELEASE_SIGNING_KEY (PEM), and writes the raw 64-byte signature beside
# it as <file>.sig. The signed message is
#   "fheuer.agentdesk.linux-artifact.v1\n<sha256 of the file>\n"
# Agent-Desk's updater (src/updater/linux.rs) and install-linux.sh verify it with the
# public key in linux-signing.pub, which this script checks every signature
# against first, so a mismatched key fails the run before anything publishes.
. "$(dirname "$0")/lib.sh"

dist=$(posix "$1")
release=$(cd "$(dirname "$0")" && pwd)
: "${AGENT_DESK_RELEASE_SIGNING_KEY:?Set AGENT_DESK_RELEASE_SIGNING_KEY}"
: "${AGENT_DESK_VERSION:?Set AGENT_DESK_VERSION}"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
(umask 077 && printf '%s\n' "$AGENT_DESK_RELEASE_SIGNING_KEY" > "$temporary/key.pem")
signed=0
for arch in x64 arm64; do
  file="$dist/hex-$AGENT_DESK_VERSION-linux-$arch"
  [ -f "$file" ] || continue
  sha=$(sha256 "$file" | cut -c1-64)
  printf 'fheuer.agentdesk.linux-artifact.v1\n%s\n' "$sha" > "$temporary/message"
  openssl pkeyutl -sign -rawin -inkey "$temporary/key.pem" -in "$temporary/message" -out "$file.sig" ||
    die "Could not sign $(basename "$file") with AGENT_DESK_RELEASE_SIGNING_KEY."
  [ "$(wc -c < "$file.sig" | tr -d ' ')" = 64 ] || die "The signature of $(basename "$file") is not 64 bytes."
  openssl pkeyutl -verify -rawin -pubin -inkey "$release/linux-signing.pub" \
    -in "$temporary/message" -sigfile "$file.sig" > /dev/null ||
    die "AGENT_DESK_RELEASE_SIGNING_KEY does not match linux-signing.pub, the key Agent-Desk trusts."
  echo "Signed $(basename "$file") ($sha)."
  signed=$((signed + 1))
done
[ "$signed" -gt 0 ] || die "No Linux executables in $dist to sign."
