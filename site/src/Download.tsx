import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react"
import type { Platform } from "./platform"
import { download, formatSize, RELEASE, type Targets } from "./release"

export function AppleMark() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" className="mark">
      <path d="M16.68 12.9c.02 2.3 2.02 3.06 2.04 3.07-.02.06-.32 1.1-1.05 2.17-.63.92-1.29 1.84-2.32 1.86-1.01.02-1.34-.6-2.5-.6-1.15 0-1.51.58-2.47.62-.99.04-1.74-.99-2.38-1.91-1.3-1.88-2.3-5.32-.96-7.65a3.7 3.7 0 0 1 3.15-1.91c.98-.02 1.91.66 2.5.66.6 0 1.72-.82 2.9-.7.49.02 1.88.2 2.77 1.5-.07.04-1.65.96-1.68 2.89ZM14.81 7.27c.53-.64.89-1.53.79-2.41-.76.03-1.68.51-2.23 1.15-.49.57-.92 1.47-.8 2.34.85.07 1.71-.43 2.24-1.08Z" />
    </svg>
  )
}

export function WindowsMark() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" className="mark">
      <path d="M4 5.6 10.5 4.7v6.6H4zM11.4 4.6 20 3.4v7.9h-8.6zM4 12.2h6.5v6.6L4 17.9zM11.4 12.2H20v7.9l-8.6-1.2z" />
    </svg>
  )
}

export function LinuxMark() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" className="mark mark--stroke">
      <path d="M5 5.5h14v13H5zM8 9l2.5 2.5L8 14M12.5 14H16" />
    </svg>
  )
}

function useCopy(): [boolean, (text: string) => void] {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 1600)
    return () => clearTimeout(timer)
  }, [copied])
  return [
    copied,
    (text) => {
      void navigator.clipboard?.writeText(text).then(() => setCopied(true))
    },
  ]
}

type Os = "mac" | "windows" | "linux"
type Arch = "x64" | "arm64"

interface Choice {
  id: string
  label: string
}

interface Row {
  label: string
  note?: string
  file?: string
  command?: string
  // Nothing to download yet: the note says so, softly.
  soon?: boolean
  // A small switch inside the row, e.g. x64/ARM64 or the Linux package format.
  choices?: Choice[]
  chosen?: string
  choose?: (id: string) => void
}

type Package = "deb" | "rpm"

interface Choices {
  arch: Arch
  setArch: (arch: Arch) => void
  pkg: Package
  setPkg: (pkg: Package) => void
}

const ARCHES: Choice[] = [
  { id: "x64", label: "x64" },
  { id: "arm64", label: "ARM64" },
]

const OSES: { id: Os; label: string; mark: ReactNode }[] = [
  { id: "mac", label: "macOS", mark: <AppleMark /> },
  { id: "windows", label: "Windows", mark: <WindowsMark /> },
  { id: "linux", label: "Linux", mark: <LinuxMark /> },
]

const SOON = "Bald verfügbar"

function rows(os: Os, intelMac: boolean, published: Targets, { arch, setArch, pkg, setPkg }: Choices): Row[] {
  const has = (target: string) => published[target] !== undefined
  const detail = (target: string, prefix?: string) => {
    const entry = published[target]
    const parts = [prefix, entry && `Version ${entry.version}`, entry && formatSize(entry.size)].filter(Boolean)
    return parts.join(" · ")
  }
  const archSwitch = { choices: ARCHES, chosen: arch, choose: (id: string) => setArch(id as Arch) }
  if (os === "mac") {
    // Pilot runs only on Apple silicon, so an Intel Mac gets the reason, not a download.
    if (intelMac) return [{ label: "Download", note: "Pilot braucht einen Mac mit Apple Silicon" }]
    if (!has("macos-arm64-dmg")) return [{ label: "Download", note: `${SOON} · Apple Silicon, macOS 15 oder neuer`, soon: true }]
    return [{ label: "Download", note: detail("macos-arm64-dmg", "Apple Silicon, macOS 15+"), file: download("macos-arm64-dmg") }]
  }
  if (os === "windows") {
    const setup = `windows-${arch}-setup`
    const any = ["x64", "arm64"].some((a) => has(`windows-${a}-setup`) || has(`windows-${a}`))
    if (!any) return [{ label: "Installer", note: `${SOON} · Windows 10 und 11`, soon: true }]
    const list: Row[] = [
      has(setup)
        ? { label: "Installer", note: detail(setup), file: download(setup), ...archSwitch }
        : { label: "Installer", note: SOON, soon: true, ...archSwitch },
    ]
    if (has(`windows-${arch}`)) list.push({ label: "PowerShell", command: RELEASE.windows.command })
    return list
  }
  const binary = `linux-${arch}`
  if (!["x64", "arm64"].some((a) => has(`linux-${a}`))) return [{ label: "Skript", note: SOON, soon: true }]
  const list: Row[] = []
  if (has(binary)) list.push({ label: "Skript", command: RELEASE.linux.command })
  const packages: (Choice & { command: string })[] = [
    ...(has(`linux-${arch}-deb`) ? [{ id: "deb", label: "Debian", command: RELEASE.linux.deb(arch) }] : []),
    ...(has(`linux-${arch}-rpm`) ? [{ id: "rpm", label: "Fedora", command: RELEASE.linux.rpm(arch) }] : []),
  ]
  const chosenPackage = packages.find((entry) => entry.id === pkg) ?? packages[0]
  if (chosenPackage)
    list.push({
      label: "Paket",
      command: chosenPackage.command,
      choices: packages,
      chosen: chosenPackage.id,
      choose: (id) => setPkg(id as Package),
    })
  list.push(
    has(binary)
      ? { label: "Binärdatei", note: detail(binary), file: download(binary), ...archSwitch }
      : { label: "Binärdatei", note: SOON, soon: true, ...archSwitch },
  )
  return list
}

function osOf(platform: Platform): Os {
  if (platform.startsWith("windows")) return "windows"
  if (platform.startsWith("linux")) return "linux"
  return "mac"
}

const archOf = (platform: Platform): Arch => (platform.endsWith("arm64") ? "arm64" : "x64")

const ICON = { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round", strokeLinejoin: "round" } as const

function CopyIcon() {
  return (
    <svg aria-hidden="true" className="stack__icon" {...ICON}>
      <rect x="8.5" y="8.5" width="11" height="11" rx="2" />
      <path d="M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2" />
    </svg>
  )
}

function CheckIcon() {
  return (
    <svg aria-hidden="true" className="stack__icon" {...ICON}>
      <path d="m5 12.5 4.5 4.5L19 7.5" />
    </svg>
  )
}

function DownloadIcon() {
  return (
    <svg aria-hidden="true" className="stack__icon" {...ICON}>
      <path d="M12 4.5v11M7 11l5 5 5-5M5 19.5h14" />
    </svg>
  )
}

function StackRow({ row }: { row: Row }) {
  const [copied, copy] = useCopy()
  return (
    <div className="stack__row">
      <span className="stack__label">{row.label}</span>
      <span className="stack__value">
        {row.choices && (
          <span className="stack__choices" role="group" aria-label={`${row.label}: Auswahl`}>
            {row.choices.map((choice) => (
              <button
                key={choice.id}
                type="button"
                aria-pressed={choice.id === row.chosen}
                onClick={() => row.choose?.(choice.id)}
              >
                {choice.label}
              </button>
            ))}
          </span>
        )}
        {row.command !== undefined ? (
          <code title={row.command}>{row.command}</code>
        ) : (
          <span className={row.soon ? "stack__soon" : undefined}>{row.note}</span>
        )}
      </span>
      {row.command === undefined && row.file === undefined ? null : row.command !== undefined ? (
        <button
          type="button"
          className="stack__action"
          onClick={() => copy(row.command!)}
          aria-label={copied ? "Kopiert" : `${row.label}-Befehl kopieren`}
          title={copied ? "Kopiert" : "Kopieren"}
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
        </button>
      ) : (
        <a
          className="stack__action stack__action--primary"
          href={row.file}
          aria-label={`${row.label} herunterladen${row.note ? ` (${row.note})` : ""}`}
          title="Herunterladen"
          download
        >
          <DownloadIcon />
        </a>
      )}
    </div>
  )
}

// Places the selected-tab pill (`.install__os::before`) under the chosen tab.
// Until the first measurement the selected tab paints its own background, so
// the prerendered page and a page without scripts still show the selection.
// The pill slides only after a click (`data-animate`), so the OS picked from
// the user agent on load doesn't animate in from macOS.
function useTabIndicator(list: RefObject<HTMLDivElement | null>, selected: Os) {
  useLayoutEffect(() => {
    const element = list.current
    if (!element) return
    const place = () => {
      const tab = element.querySelector<HTMLElement>('[aria-selected="true"]')
      if (!tab) return
      element.style.setProperty("--tab-x", `${tab.offsetLeft}px`)
      element.style.setProperty("--tab-width", `${tab.offsetWidth}px`)
      element.dataset.measured = ""
    }
    place()
    // Tab widths change when Inter finishes loading or the viewport resizes.
    const observer = new ResizeObserver(place)
    observer.observe(element)
    return () => observer.disconnect()
  }, [list, selected])
}

export default function Download({ platform, published }: { platform: Platform; published: Targets }) {
  const [chosenOs, setOs] = useState<Os>()
  const os = chosenOs ?? osOf(platform)
  const [chosenArch, setArch] = useState<Arch>()
  const arch = chosenArch ?? archOf(platform)
  const [pkg, setPkg] = useState<Package>("deb")
  const choices = { arch, setArch, pkg, setPkg }
  const tabs = useRef<HTMLDivElement>(null)
  useTabIndicator(tabs, os)

  return (
    <div className="hero__action install" id="download">
      <div className="install__os" role="tablist" aria-label="Betriebssystem" ref={tabs}>
        {OSES.map((entry) => (
          <button
            key={entry.id}
            id={`install-tab-${entry.id}`}
            type="button"
            role="tab"
            aria-selected={entry.id === os}
            aria-controls={`install-panel-${entry.id}`}
            onClick={() => {
              if (tabs.current) tabs.current.dataset.animate = ""
              setOs(entry.id)
            }}
          >
            {entry.mark}
            <span>{entry.label}</span>
          </button>
        ))}
      </div>
      {/* Every OS's stack shares one grid cell, so the tallest reserves the
          height and switching tabs never moves the page. */}
      <div className="install__panels">
        {OSES.map((entry) => (
          <div
            key={entry.id}
            id={`install-panel-${entry.id}`}
            className="stack"
            role="tabpanel"
            aria-labelledby={`install-tab-${entry.id}`}
            data-active={entry.id === os ? "" : undefined}
          >
            {rows(entry.id, platform === "mac-intel", published, choices).map((row) => (
              <StackRow key={row.label} row={row} />
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}
