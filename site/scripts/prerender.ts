// Puts the server-rendered page into the built index.html, so the copy and
// the download switcher are in the HTML before JavaScript runs; main.tsx
// hydrates it. Stamps the origin into every absolute URL.
import { readFileSync, rmSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { render } from "../dist-server/prerender.js"
import { ORIGIN } from "../src/base"
import { RELEASE } from "../src/release"

const root = resolve(import.meta.dirname, "..")
const dist = join(root, "dist")
const index = join(dist, "index.html")
const html = readFileSync(index, "utf8")
const marker = '<div id="root"></div>'
if (!html.includes(marker)) throw new Error(`index.html has no ${marker}`)
writeFileSync(index, html.replace(marker, `<div id="root">${render()}</div>`).replaceAll("__PILOT_VERSION__", RELEASE.version ?? ""))
for (const name of ["index.html", "llms.txt", "robots.txt", "sitemap.xml", "versionen/index.html"]) {
  const path = join(dist, name)
  writeFileSync(path, readFileSync(path, "utf8").replaceAll("__ORIGIN__", ORIGIN))
}
rmSync(join(root, "dist-server"), { recursive: true, force: true })
