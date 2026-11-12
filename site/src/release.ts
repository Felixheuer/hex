// What the download switcher offers: the targets scripts/pilot-publish.sh
// has published (/download/index.json), as recorded at build time in
// release-data.json and refreshed by the page on load. A target that isn't
// published yet shows "Bald verfügbar" instead of a dead link.
import { useEffect, useState } from "react"
import { DOWNLOAD_INDEX, DOWNLOAD_PATH, ORIGIN } from "./base"
import data from "./release-data.json"

export interface Published {
  version: string
  size: number
  sha256: string
  published_at: string
}

export type Targets = Record<string, Published>

const BUILT: Targets = (data as { targets: Targets }).targets ?? {}

export const download = (target: string) => `${DOWNLOAD_PATH}/${target}`

const powershell = `powershell -ExecutionPolicy Bypass -Command "iwr ${ORIGIN}/install-windows.ps1 -OutFile install-windows.ps1; ./install-windows.ps1"`

export const RELEASE = {
  version: newest(BUILT),
  linux: {
    command: `curl -fsSL ${ORIGIN}/install-linux.sh | sh`,
    deb: (arch: string) => `curl -fLo /tmp/hex.deb ${ORIGIN}${download(`linux-${arch}-deb`)} && sudo apt install /tmp/hex.deb`,
    rpm: (arch: string) => `curl -fLo /tmp/hex.rpm ${ORIGIN}${download(`linux-${arch}-rpm`)} && sudo dnf install /tmp/hex.rpm`,
  },
  windows: { command: powershell },
} as const

/** The newest version among the published targets, for the label. */
export function newest(targets: Targets): string | undefined {
  return Object.values(targets).sort((a, b) => b.published_at.localeCompare(a.published_at))[0]?.version
}

/** The published targets: the build's, then the live index once it loads. */
export function usePublished(): Targets {
  const [targets, setTargets] = useState<Targets>(BUILT)
  useEffect(() => {
    let current = true
    fetch(DOWNLOAD_INDEX, { cache: "no-cache" })
      .then((response) => (response.ok ? response.json() : undefined))
      .then((value: { targets?: Targets } | undefined) => {
        if (current && value?.targets) setTargets(value.targets)
      })
      .catch(() => {})
    return () => {
      current = false
    }
  }, [])
  return targets
}

export function formatSize(bytes: number): string {
  return `${(bytes / 1024 / 1024).toFixed(0)} MB`
}
