#!/usr/bin/env bash
# Publishes Pilot release files on this server (ai.sms-pilot.de) in the
# layout Hex's updater reads (docs/pilot-release.md,
# docs/plans/release-service.md):
#
#   <root>/release/hex/desktop/<channel>.json     the manifest the app polls
#   <root>/release/hex/desktop/archive/           every manifest ever published
#   <root>/release/hex/files/<sha256>/<name>      the files, content-addressed
#   <root>/release/hex/history.json               the version history (site /versionen)
#   <root>/download/<target>                      the newest file per target (site links)
#   <root>/download/index.json                    which targets the site offers
#
# Every target gets an ed25519 signature by the release key over
# "pilot.release-artifact.v1\n<sha256>\n" as <target>-sig, which the updater
# requires before it fetches the file. The private key never leaves this
# server.
#
#   pilot-publish.sh publish [options] <version> <file>...
#   pilot-publish.sh rollback [--channel stable]
#   pilot-publish.sh status [--channel stable]
#
# publish options:
#   --channel stable|beta|dev   (default stable)
#   --notes <file>              plain-text notes for the version history
#   --rollout <0-100>           rollout percent (default 100)
#   --required                  installs take it without the rollout
#
# Files are recognized by their build names: hex-<v>-linux-x64[.deb|.rpm],
# hex-<v>-linux-arm64[.deb|.rpm], hex-<v>-macos-arm64.zip|.dmg,
# hex-<v>-windows-x64.exe|.com, hex-setup-<v>-windows-x64.exe (and arm64).
# Publishing the version the channel already carries adds or replaces those
# targets and keeps the others (Linux, Mac, and Windows built on different
# machines join one release); a new version starts a release with only the
# files given. The previous manifest stays in archive/ for `rollback`.
#
# Environment: PILOT_ROOT (default /srv/pilot), PILOT_SIGNING_KEY (default
# ~/.config/pilot-release/signing-key), PILOT_SITE (https://ai.sms-pilot.de).
set -euo pipefail
umask 022

root=${PILOT_ROOT:-/srv/pilot}
key=${PILOT_SIGNING_KEY:-$HOME/.config/pilot-release/signing-key}
site=${PILOT_SITE:-https://ai.sms-pilot.de}
release="$root/release/hex"
work=
trap '[ -z "$work" ] || rm -rf "$work"' EXIT

die() {
  echo "pilot-publish: $*" >&2
  exit 1
}

for command in jq openssl sha256sum flock; do
  command -v "$command" > /dev/null 2>&1 || die "requires $command"
done

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed -e '/^set -euo/d' -e 's/^# \{0,1\}//' >&2
  exit 1
}

# The target and published file name of a build file, or nothing.
target_of() {
  local name=$1 v=$2 arch
  for arch in x64 arm64; do
    case "$name" in
      "hex-$v-linux-$arch") printf 'linux-%s hex\n' "$arch" && return ;;
      "hex-$v-linux-$arch.deb") printf 'linux-%s-deb hex.deb\n' "$arch" && return ;;
      "hex-$v-linux-$arch.rpm") printf 'linux-%s-rpm hex.rpm\n' "$arch" && return ;;
      "hex-$v-windows-$arch.exe") printf 'windows-%s hex.exe\n' "$arch" && return ;;
      "hex-$v-windows-$arch.com") printf 'windows-%s-cli hex.com\n' "$arch" && return ;;
      "hex-setup-$v-windows-$arch.exe") printf 'windows-%s-setup PilotSetup.exe\n' "$arch" && return ;;
    esac
  done
  case "$name" in
    "hex-$v-macos-arm64.zip") printf 'macos-arm64 Hex.zip\n' ;;
    "hex-$v-macos-arm64.dmg") printf 'macos-arm64-dmg Pilot.dmg\n' ;;
  esac
}

sha256() {
  sha256sum "$1" | awk '{ print $1 }'
}

# Writes <path> from stdin through a temporary file in the same directory, so
# readers see the old or the new file, never a partial one.
write_atomic() {
  local path=$1 partial
  partial=$(mktemp "$(dirname "$path")/.$(basename "$path").XXXXXX")
  cat > "$partial"
  chmod 644 "$partial"
  mv -f "$partial" "$path"
}

# Stores <file> as files/<sha256>/<name> unless it is already there; prints
# its absolute-path URL.
store() {
  local file=$1 name=$2 sha dir
  sha=$(sha256 "$file")
  dir="$release/files/$sha"
  mkdir -p "$dir"
  if [ ! -f "$dir/$name" ]; then
    cp "$file" "$dir/.$name.partial"
    chmod 644 "$dir/.$name.partial"
    mv -f "$dir/.$name.partial" "$dir/$name"
  fi
  [ "$(sha256 "$dir/$name")" = "$sha" ] || die "files/$sha/$name has other bytes"
  printf '/release/hex/files/%s/%s\n' "$sha" "$name"
}

# Signs the SHA-256 <sha> into <out> (64 bytes).
sign() {
  local sha=$1 out=$2 message
  message=$(mktemp)
  printf 'pilot.release-artifact.v1\n%s\n' "$sha" > "$message"
  openssl pkeyutl -sign -rawin -inkey "$key" -in "$message" -out "$out"
  rm -f "$message"
  [ "$(wc -c < "$out" | tr -d ' ')" = 64 ] || die "the signature is not 64 bytes"
}

# Points download/<target> at the stored file (a relative symlink, switched
# atomically) and records it in download/index.json.
link_download() {
  local target=$1 url=$2 version=$3 size=$4 sha=$5 link partial
  mkdir -p "$root/download"
  link="$root/download/$target"
  partial="$root/download/.$target.partial"
  rm -f "$partial"
  ln -s "..$url" "$partial"
  mv -Tf "$partial" "$link"
  local index="$root/download/index.json"
  [ -f "$index" ] || echo '{"targets":{}}' > "$index"
  jq --arg target "$target" --arg version "$version" --arg url "/download/$target" \
    --argjson size "$size" --arg sha256 "$sha" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '.targets[$target] = {version: $version, url: $url, size: $size, sha256: $sha256, published_at: $at}' \
    "$index" | write_atomic "$index"
}

# The command a .deb or .rpm install shows when a newer package is out.
package_distributions() {
  local manifest=$1 arch
  for arch in x64 arm64; do
    if jq -e --arg t "linux-$arch-deb" '.release.targets[$t]' "$manifest" > /dev/null; then
      jq -n --arg arch "$arch" --arg v "$(jq -r .release.version "$manifest")" --arg site "$site" \
        '{kind: "deb-\($arch)", package: "hex", version: $v,
          command: "curl -fLo /tmp/hex.deb \($site)/download/linux-\($arch)-deb && sudo apt install /tmp/hex.deb"}'
    fi
    if jq -e --arg t "linux-$arch-rpm" '.release.targets[$t]' "$manifest" > /dev/null; then
      jq -n --arg arch "$arch" --arg v "$(jq -r .release.version "$manifest")" --arg site "$site" \
        '{kind: "rpm-\($arch)", package: "hex", version: $v,
          command: "curl -fLo /tmp/hex.rpm \($site)/download/linux-\($arch)-rpm && sudo dnf install /tmp/hex.rpm"}'
    fi
  done
}

lock() {
  mkdir -p "$release/desktop/archive" "$release/files"
  exec 9> "$release/.publish.lock"
  flock -w 600 9 || die "another publish is running"
}

next_sequence() {
  local channel=$1 current
  current=$(jq -r '.sequence // 0' "$release/desktop/$channel.json" 2>/dev/null || echo 0)
  local archived
  archived=$(find "$release/desktop/archive" -name "$channel-*.json" 2>/dev/null \
    | sed -E "s#.*/$channel-0*([0-9]+)\\.json#\\1#" | sort -n | tail -1)
  archived=${archived:-0}
  [ "$archived" -gt "$current" ] && current=$archived
  echo $((current + 1))
}

# Puts <manifest> live as the channel's manifest and archives it.
go_live() {
  local channel=$1 manifest=$2 sequence
  sequence=$(jq -r .sequence "$manifest")
  cp "$manifest" "$release/desktop/archive/$channel-$(printf '%06d' "$sequence").json"
  write_atomic "$release/desktop/$channel.json" < "$manifest"
}

record_history() {
  local channel=$1 manifest=$2 notes=${3:-} history="$release/history.json" notes_text=""
  [ -f "$history" ] || echo '{"versions":[]}' > "$history"
  if [ -n "$notes" ]; then
    notes_text=$(head -c 20000 "$notes")
  fi
  jq --arg channel "$channel" --arg notes "$notes_text" --slurpfile m "$manifest" '
    ($m[0].release) as $r
    | ([.versions[] | select(.version == $r.version and .channel == $channel)][0]) as $old
    | {version: $r.version, channel: $channel,
       published_at: ($old.published_at // $m[0].published_at),
       updated_at: $m[0].published_at,
       notes: (if $notes != "" then $notes else ($old.notes // "") end),
       targets: ([$r.targets | keys[] | select(endswith("-sig") | not)])} as $entry
    | .versions = ([$entry] + [.versions[] | select((.version == $r.version and .channel == $channel) | not)])
  ' "$history" | write_atomic "$history"
}

publish() {
  local channel=stable notes="" rollout=100 required=false
  while [ $# -gt 0 ]; do
    case "$1" in
      --channel) channel=$2; shift 2 ;;
      --notes) notes=$2; shift 2 ;;
      --rollout) rollout=$2; shift 2 ;;
      --required) required=true; shift ;;
      --) shift; break ;;
      -*) usage ;;
      *) break ;;
    esac
  done
  [ $# -ge 2 ] || usage
  local version=$1
  shift
  case "$channel" in stable | beta | dev) ;; *) die "unknown channel $channel" ;; esac
  printf '%s\n' "$version" | grep -Eq '^[0-9A-Za-z][0-9A-Za-z.+_-]{0,127}$' || die "invalid version $version"
  printf '%s\n' "$rollout" | grep -Eq '^([0-9]|[1-9][0-9]|100)$' || die "rollout must be 0-100"
  [ -z "$notes" ] || [ -f "$notes" ] || die "no notes file $notes"
  [ -r "$key" ] || die "no signing key at $key"
  [ "$(stat -c %a "$key")" = 600 ] || die "$key must be mode 600"
  lock

  work=$(mktemp -d)
  echo '{}' > "$work/targets.json"
  local current="$release/desktop/$channel.json"
  if [ -f "$current" ] && [ "$(jq -r .release.version "$current")" = "$version" ]; then
    jq '.release.targets' "$current" > "$work/targets.json"
    echo "Adding to $channel $version (keeping $(jq -r 'keys | join(", ")' "$work/targets.json"))." >&2
  fi
  # Every archived release of this version on any channel must agree on
  # each target's bytes: a version is never published twice with others.
  local file name target published url sha size sig sig_url sig_sha
  for file in "$@"; do
    [ -f "$file" ] || die "no file $file"
    name=$(basename "$file")
    target='' published=''
    read -r target published < <(target_of "$name" "$version") || true
    [ -n "${target:-}" ] || die "$name is not a Pilot $version release file"
    sha=$(sha256 "$file")
    size=$(wc -c < "$file" | tr -d ' ')
    [ "$size" -gt 0 ] || die "$name is empty"
    local earlier
    earlier=$(jq -r --arg v "$version" --arg t "$target" \
      'select(.release.version == $v) | .release.targets[$t].sha256 // empty' \
      "$release"/desktop/archive/*.json 2>/dev/null | sort -u | head -1 || true)
    if [ -n "$earlier" ] && [ "$earlier" != "$sha" ]; then
      die "$target of $version was already published with other bytes; publish a new version"
    fi
    url=$(store "$file" "$published")
    sig="$work/$target.sig"
    sign "$sha" "$sig"
    sig_sha=$(sha256 "$sig")
    sig_url=$(store "$sig" "${published%.*}.sig")
    jq --arg t "$target" --arg url "$url" --argjson size "$size" --arg sha "$sha" \
      --arg st "$target-sig" --arg surl "$sig_url" --arg ssha "$sig_sha" \
      '.[$t] = {url: $url, size: $size, sha256: $sha} | .[$st] = {url: $surl, size: 64, sha256: $ssha}' \
      "$work/targets.json" > "$work/targets.next" && mv "$work/targets.next" "$work/targets.json"
    printf '%s\t%s\t%s\t%s\n' "$target" "$url" "$size" "$sha" >> "$work/links"
    echo "  $target  $published  $size bytes  sha256 $sha" >&2
  done

  local sequence now
  sequence=$(next_sequence "$channel")
  now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  jq -n --arg channel "$channel" --argjson sequence "$sequence" --arg now "$now" \
    --arg version "$version" --arg notes "$site/versionen#$version" \
    --argjson rollout "$rollout" --argjson required "$required" --slurpfile targets "$work/targets.json" \
    '{schema: 1, app: "hex", artifact: "desktop", channel: $channel, sequence: $sequence,
      published_at: $now,
      release: {version: $version, notes_url: $notes, rollout_percent: $rollout, required: $required,
                targets: $targets[0], distributions: [{kind: "direct"}], revoked_versions: []}}' \
    > "$work/manifest.json"
  package_distributions "$work/manifest.json" | jq -s '.' > "$work/packages.json"
  jq --slurpfile p "$work/packages.json" '.release.distributions += $p[0]' "$work/manifest.json" > "$work/manifest.next"
  mv "$work/manifest.next" "$work/manifest.json"
  verify_manifest "$work/manifest.json"
  go_live "$channel" "$work/manifest.json"
  # Download links follow the stable channel only.
  if [ "$channel" = stable ]; then
    while IFS=$'\t' read -r target url size sha; do
      link_download "$target" "$url" "$version" "$size" "$sha"
    done < "$work/links"
  fi
  record_history "$channel" "$work/manifest.json" "$notes"
  echo "Published Pilot $version on $channel (sequence $sequence): $site/release/hex/desktop/$channel.json" >&2
}

# Checks every target's file, size, SHA-256, and signature on disk against
# the public half of the key.
verify_manifest() {
  local manifest=$1 public message target url size sha sig_url
  public=$(mktemp)
  message=$(mktemp)
  openssl pkey -in "$key" -pubout -out "$public" 2> /dev/null || cp "${PILOT_PUBLIC_KEY:?}" "$public"
  [ "$(jq -r '"\(.schema) \(.app) \(.artifact)"' "$manifest")" = "1 hex desktop" ] || die "malformed manifest"
  while IFS=$'\t' read -r target url size sha; do
    case "$target" in *-sig) continue ;; esac
    [ -f "$root$url" ] || die "$target: $url is missing"
    [ "$(wc -c < "$root$url" | tr -d ' ')" = "$size" ] || die "$target: wrong size"
    [ "$(sha256 "$root$url")" = "$sha" ] || die "$target: wrong sha256"
    sig_url=$(jq -er --arg t "$target-sig" '.release.targets[$t].url' "$manifest") || die "$target has no signature"
    printf 'pilot.release-artifact.v1\n%s\n' "$sha" > "$message"
    openssl pkeyutl -verify -rawin -pubin -inkey "$public" -in "$message" -sigfile "$root$sig_url" > /dev/null \
      || die "$target: the signature does not verify"
  done < <(jq -r '.release.targets | to_entries[] | [.key, .value.url, .value.size, .value.sha256] | @tsv' "$manifest")
  rm -f "$public" "$message"
}

rollback() {
  local channel=stable
  while [ $# -gt 0 ]; do
    case "$1" in
      --channel) channel=$2; shift 2 ;;
      *) usage ;;
    esac
  done
  lock
  local current="$release/desktop/$channel.json" version previous
  [ -f "$current" ] || die "$channel has no release"
  version=$(jq -r .release.version "$current")
  # The newest archived manifest of another version.
  previous=$(for file in $(find "$release/desktop/archive" -name "$channel-*.json" | sort -r); do
    if [ "$(jq -r .release.version "$file")" != "$version" ]; then
      echo "$file"
      break
    fi
  done)
  [ -n "$previous" ] || die "no earlier version of $channel to roll back to"
  work=$(mktemp -d)
  jq --argjson sequence "$(next_sequence "$channel")" --arg now "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '.sequence = $sequence | .published_at = $now' "$previous" > "$work/manifest.json"
  verify_manifest "$work/manifest.json"
  go_live "$channel" "$work/manifest.json"
  if [ "$channel" = stable ]; then
    while IFS=$'\t' read -r target url size sha; do
      link_download "$target" "$url" "$(jq -r .release.version "$work/manifest.json")" "$size" "$sha"
    done < <(jq -r '.release.targets | to_entries[] | select(.key | endswith("-sig") | not) | [.key, .value.url, .value.size, .value.sha256] | @tsv' "$work/manifest.json")
  fi
  echo "Rolled $channel back from $version to $(jq -r .release.version "$work/manifest.json")." >&2
}

status() {
  local channel=stable
  [ "${1:-}" = --channel ] && channel=$2
  local current="$release/desktop/$channel.json"
  [ -f "$current" ] || die "$channel has no release"
  jq -r '"\(.channel) \(.release.version) (sequence \(.sequence), \(.published_at))",
    (.release.targets | to_entries[] | select(.key | endswith("-sig") | not) | "  \(.key)  \(.value.size) bytes  \(.value.url)")' "$current"
}

command=${1:-}
[ -n "$command" ] && shift || usage
case "$command" in
  publish) publish "$@" ;;
  rollback) rollback "$@" ;;
  status) status "$@" ;;
  *) usage ;;
esac
