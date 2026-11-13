#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
installer="$root/.github/release/install-linux.sh"

if [ "${1:-install}" = uninstall ]; then
  exec "$installer" uninstall
fi
if [ "${1:-install}" != install ]; then
  echo "Usage: $0 [install|uninstall]" >&2
  exit 1
fi

target_dir=${CARGO_TARGET_DIR:-"$root/target"}
cargo build --locked --release --manifest-path "$root/Cargo.toml"
version=$("$target_dir/release/hex" --version | awk 'NF { value=$NF } END { print value }')

AGENT_DESK_INSTALL_SOURCE=1 \
AGENT_DESK_INSTALL_ARTIFACT="$target_dir/release/hex" \
AGENT_DESK_INSTALL_VERSION="$version" \
  exec "$installer"
