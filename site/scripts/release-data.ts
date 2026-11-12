// Writes src/release-data.json, which targets the download switcher offers
// at build time (the page refreshes it from /download/index.json on load),
// and copies the installers and the license into public/.
//
//   bun scripts/release-data.ts
//
// PILOT_DOWNLOAD_INDEX names a local index.json (the deploy script passes
// /srv/pilot/download/index.json); otherwise the live one is fetched, and a
// missing one means nothing is published yet.
import { copyFileSync, existsSync, readFileSync, writeFileSync } from "node:fs"
import { DOWNLOAD_INDEX, ORIGIN } from "../src/base"

const root = new URL("..", import.meta.url)
type Index = { targets: Record<string, { version: string; size: number; sha256: string; published_at: string }> }

async function index(): Promise<Index> {
  const local = process.env.PILOT_DOWNLOAD_INDEX
  if (local) return existsSync(local) ? (JSON.parse(readFileSync(local, "utf8")) as Index) : { targets: {} }
  const response = await fetch(`${ORIGIN}${DOWNLOAD_INDEX}`, { headers: { "User-Agent": "pilot-site" } })
  if (response.status === 404) return { targets: {} }
  if (!response.ok) throw new Error(`${DOWNLOAD_INDEX} answered ${response.status}`)
  return (await response.json()) as Index
}

const data = await index()
writeFileSync(new URL("src/release-data.json", root), `${JSON.stringify(data, null, 2)}\n`)
copyFileSync(new URL("../.github/release/install-linux.sh", root), new URL("public/install-linux.sh", root))
copyFileSync(new URL("../.github/release/install-windows.ps1", root), new URL("public/install-windows.ps1", root))
copyFileSync(new URL("../LICENSE", root), new URL("public/LICENSE.txt", root))
const targets = Object.keys(data.targets)
console.log(`Download switcher offers: ${targets.length ? targets.join(", ") : "nothing yet"}.`)
