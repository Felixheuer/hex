#!/bin/sh
# Builds the website (site/, static, base /) and puts it live at
# /srv/pilot/site, which Caddy serves as https://ai.sms-pilot.de
# (docs/pilot-release.md). Each build lands in /srv/pilot/sites/<time> and
# /srv/pilot/site switches to it atomically; the previous two stay for
# rollback (`ln -sfn sites/<older> /srv/pilot/site.next && mv -T ...`).
# It also installs scripts/pilot-publish.sh as /srv/pilot/bin/pilot-publish.sh,
# which the Mac and Windows release scripts run over ssh.
#
#   scripts/pilot-site-deploy.sh
set -eu
umask 022

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
pilot_root=${PILOT_ROOT:-/srv/pilot}
bun=${BUN:-$(command -v bun || echo /home/t3/.npm-global/bin/bun)}
[ -x "$bun" ] || {
  echo "The site build needs bun." >&2
  exit 1
}

(
  cd "$root/site"
  "$bun" install --frozen-lockfile >&2
  PILOT_DOWNLOAD_INDEX="$pilot_root/download/index.json" "$bun" run build >&2
)

mkdir -p "$pilot_root/bin"
install -m 755 "$root/scripts/pilot-publish.sh" "$pilot_root/bin/pilot-publish.sh"

stamp=$(date -u +%Y%m%d-%H%M%S)
mkdir -p "$pilot_root/sites"
target="$pilot_root/sites/$stamp"
cp -R "$root/site/dist" "$target.partial"
chmod -R go-w,a+rX "$target.partial"
mv "$target.partial" "$target"
# A first deploy replaces the placeholder directory.
if [ -d "$pilot_root/site" ] && [ ! -L "$pilot_root/site" ]; then
  mv "$pilot_root/site" "$pilot_root/sites/placeholder-$stamp"
fi
ln -sfn "sites/$stamp" "$pilot_root/site.next"
mv -T "$pilot_root/site.next" "$pilot_root/site"
# Keep the newest three builds.
ls -1d "$pilot_root"/sites/2* 2> /dev/null | sort -r | tail -n +4 | while read -r old; do
  rm -rf "$old"
done
echo "Site live from $target" >&2
