import { useLayoutEffect, useState } from "react"
import Download from "./Download"
import { guessPlatform, refinePlatform, type Platform } from "./platform"
import { newest, usePublished } from "./release"

export function PilotMark({ className = "wordmark__icon" }: { className?: string }) {
  return (
    <svg aria-hidden="true" viewBox="0 0 64 64" className={className}>
      <rect x="5" y="5" width="54" height="54" rx="17" />
      <path d="M21 27v10M28 21v22M35 25v14M42 29v6" />
    </svg>
  )
}

const FEATURES = [
  {
    title: "Diktieren, überall",
    text: "Halte die Diktat-Taste, sprich, lass los: Pilot schreibt deinen Text genau dort, wo dein Cursor steht – in jeder App. Die Spracherkennung läuft lokal auf deinem Gerät.",
  },
  {
    title: "Ein Assistent, der zuhört",
    text: "Halte die Aktions-Taste und sag, was du brauchst. Pilot antwortet, erledigt es direkt oder startet eine Aufgabe im Hintergrund – und hört danach für eine Rückfrage weiter zu.",
  },
  {
    title: "Immer aktuell",
    text: "Pilot prüft selbst auf Updates, lädt sie geprüft und signiert herunter und installiert sie mit einem Klick auf „Neu starten“.",
  },
]

export default function App() {
  // The prerendered HTML is the Mac page; switch before the first paint, then
  // refine with client hints.
  const [platform, setPlatform] = useState<Platform>("mac")
  useLayoutEffect(() => {
    let current = true
    const guess = guessPlatform()
    setPlatform(guess)
    void refinePlatform(guess).then((refined) => {
      if (current && refined !== guess) setPlatform(refined)
    })
    return () => {
      current = false
    }
  }, [])
  const published = usePublished()
  const version = newest(published)

  return (
    <>
      <main className="page">
        <header className="header">
          <a className="wordmark" href="/" aria-label="Pilot – Startseite">
            <PilotMark />
            <span>Pilot</span>
          </a>
          <nav className="header__nav" aria-label="Seiten">
            <a href="#download">Download</a>
            <a href="/versionen/">Versionen</a>
          </nav>
        </header>

        <section className="hero">
          <div className="hero__copy">
            <p className="hero__eyebrow">Pilot – dein Sprach-Assistent</p>
            <h1>Sprich. Pilot erledigt den Rest.</h1>
            <p>
              Diktieren in jeder App und ein KI-Assistent, mit dem du einfach redest. Für macOS, Windows und Linux.
            </p>
          </div>

          <Download platform={platform} published={published} />
          <p className="hero__version">{version ? `Aktuelle Version: ${version}` : " "}</p>
        </section>

        <section className="features" aria-label="Was Pilot kann">
          {FEATURES.map((feature) => (
            <article key={feature.title} className="feature">
              <h2>{feature.title}</h2>
              <p>{feature.text}</p>
            </article>
          ))}
        </section>
      </main>
      <footer className="site-footer">
        <p>
          Pilot basiert auf <a href="https://github.com/anomalyco/hex">Hex</a> von Kit Langton und Anomaly, veröffentlicht unter der{" "}
          <a href="/LICENSE.txt">MIT-Lizenz</a>.
        </p>
        <p>
          <a href="/versionen/">Versionen</a> · <a href="/release/hex/desktop/stable.json">Update-Kanal</a>
        </p>
      </footer>
    </>
  )
}
