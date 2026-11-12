// Pilot's website, downloads, and release server: one host, served by Caddy
// from /srv/pilot (docs/pilot-release.md).
export const ORIGIN = "https://ai.sms-pilot.de"
// /download/<target> is the newest published file of a target (a symlink
// scripts/pilot-publish.sh switches); index.json says which targets exist.
export const DOWNLOAD_PATH = "/download"
export const DOWNLOAD_INDEX = `${DOWNLOAD_PATH}/index.json`
