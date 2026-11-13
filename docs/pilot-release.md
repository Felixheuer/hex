# Pilot: Website, Downloads und Update-Kanal

Pilot ist der Name der Website und der Downloads; die App heißt **Agent-Desk**
(ein Fork von Hex, `anomalyco/hex`): Binary und Crate `agent-desk`, Bundle-ID
`fheuer.agentdesk` (Dev `fheuer.agentdesk.dev`), URL-Schema `agent-desk://`,
Datenordner `~/Library/Application Support/agent-desk`, `~/.config/agent-desk`,
`~/.local/share/agent-desk`, `%APPDATA%\agent-desk`, Umgebungsvariablen
`AGENT_DESK_*`. Der Pfad des Update-Kanals bleibt `release/hex/`: `hex` ist die
App-ID des Release-Protokolls, die jede installierte App und jede
Installations-Quittung nennt.

## Umzug von Hex zu Agent-Desk (einmalig, entfernen nach 2026-12)

Nur diese einmaligen Migrationen gibt es, keine Aliase (kein `hex`-Befehl,
kein `hex://`, keine `HEX_*`-Variablen):

- **Datenordner** (`src/app_paths.rs`, `take_over`): beim ersten Start wird
  `~/Library/Application Support/hex` (bzw. `hex-dev`) in `agent-desk`
  (`agent-desk-dev`) umbenannt und `~/.config/hex` in `~/.config/agent-desk` –
  nur wenn der neue Ordner fehlt und kein laufendes Hex seine Locks hält;
  sonst bleiben beide unberührt. Dabei werden umbenannte Schlüssel in
  `settings.json` einmal auf die neuen Namen geschrieben. Linux und Windows
  benutzen den alten Ordner an Ort und Stelle (die Installation liegt darin).
- **Schlüsselbund**: API-Schlüssel unter „Hex Assistant“ ziehen beim ersten
  Lesen nach „Agent-Desk Assistant“ um.
- **App**: Ein installiertes Hex nimmt nur Updates mit seiner Bundle-ID an.
  Darum trägt jedes Release zusätzlich die **Brücke**: `hex-<v>-macos-arm64.zip`,
  ein `Hex.app` mit `anomaly.hex`, das `Agent-Desk.app` in
  `Contents/Library` mitbringt (Target `macos-arm64`, das alte Hex-Installationen
  abfragen). Agent-Desk selbst fragt `macos-arm64-agentdesk` ab. Gestartet
  legt die Brücke `Agent-Desk.app` daneben, öffnet es und beendet sich;
  Agent-Desk entfernt danach die Brücke und Hex' Update-Reste
  (`src/updater/macos.rs`). `--install` entfernt ein altes
  `/Applications/Hex.app` direkt.
- **Berechtigungen**: macOS gibt der neuen Bundle-ID keine der alten
  Freigaben. Einstellungen von vor dem Umzug öffnen die Einrichtung einmal
  wieder, wenn etwas fehlt (`onboarding::recheck_after_identity_change`).

Alles läuft auf diesem Server (`ai.sms-pilot.de`, 91.99.118.129) mit Caddy:

```
https://ai.sms-pilot.de/                            Website (site/, statisch)
https://ai.sms-pilot.de/versionen/                  Versionsliste (aus history.json)
https://ai.sms-pilot.de/download/<target>           neueste Datei je Target (Symlink)
https://ai.sms-pilot.de/download/index.json         welche Targets es gibt (Website liest das)
https://ai.sms-pilot.de/install-linux.sh            Linux-Installer (curl | sh)
https://ai.sms-pilot.de/install-windows.ps1         Windows-Installer per PowerShell
https://ai.sms-pilot.de/release/hex/desktop/stable.json   Manifest, das die App abfragt
https://ai.sms-pilot.de/release/hex/files/<sha256>/<name> die Dateien, inhaltsadressiert
```

Auf der Platte (`/srv/pilot`, gehört `t3`):

```
/srv/pilot/site -> sites/<zeit>     aktuelle Website (pilot-site-deploy.sh), 3 Builds bleiben
/srv/pilot/release/hex/desktop/     <kanal>.json + archive/<kanal>-<sequenz>.json (jedes Manifest)
/srv/pilot/release/hex/files/       Release-Dateien und ihre .sig, nie überschrieben
/srv/pilot/release/hex/history.json Versionsliste
/srv/pilot/download/                Symlinks auf die neueste Datei je Target, index.json
/srv/pilot/bin/pilot-publish.sh     das Publish-Skript (für ssh von Mac/Windows)
/srv/pilot/incoming/                Uploads, bevor sie veröffentlicht sind (0700)
```

Caddy: `/etc/caddy/caddy-ai-sms-pilot.conf` (Quelle im Repo:
`packaging/pilot/caddy-ai-sms-pilot.conf`), importiert von
`/etc/caddy/caddy-static-sites.conf`. Ändern:

```sh
sudo cp /etc/caddy/caddy-ai-sms-pilot.conf /etc/caddy/caddy-ai-sms-pilot.conf.bak-$(date +%Y%m%d)
sudo install -m 644 packaging/pilot/caddy-ai-sms-pilot.conf /etc/caddy/caddy-ai-sms-pilot.conf
# mit der Umgebung des Dienstes prüfen (CF_API_TOKEN für TLS), dann neu laden:
sudo bash -c 'set -a; while IFS= read -r l; do case "$l" in Environment=*) v=${l#Environment=}; v=${v#\"}; v=${v%\"}; export "$v";; esac; done < /etc/systemd/system/caddy.service.d/override.conf; caddy validate --config /etc/caddy/Caddyfile'
sudo systemctl reload caddy
```

## Wie die App updatet

Unverändert das Protokoll von Agent-Desk (`docs/plans/release-service.md`), nur
selbst gehostet: `ORIGIN` in `src/updater.rs` ist
`https://ai.sms-pilot.de/release`.

- Prüfung 10 s nach dem Start, dann stündlich und bei „Nach Updates suchen“.
- Die App lädt das Target ihres Systems, prüft Größe, SHA-256 und die
  **ed25519-Signatur von Pilots Release-Schlüssel** (`<target>-sig`,
  `src/updater/signature.rs`), bereitet das Update vor und zeigt
  „Neu starten, um zu aktualisieren“. Ein Klick startet in die neue Version.
- Zusätzlich: macOS prüft die Code-Signatur (Bundle-ID `fheuer.agentdesk`, Team aus
  `AGENT_DESK_UPDATE_TEAM_ID`, das `pilot-release-macos.sh` beim Bauen einstempelt);
  Windows prüft Authenticode nur, wenn der Build einen Signierer kennt
  (`AGENT_DESK_AUTHENTICODE_SIGNER`, gesetzt von `pilot-release-windows.ps1
  -CertificateThumbprint`).
- Versionen werden nie verglichen: Linux und Windows erkennen den laufenden
  Build an der SHA-256 der Datei, macOS an der exakten Version. Was im
  Manifest steht, wird installiert – auch ein Rollback.

**Live und Dev (macOS).** Auf dem Mac stehen genau zwei Agent-Desk-Apps:

- **Live**: `/Applications/Agent-Desk.app` (`fheuer.agentdesk`), der veröffentlichte
  Pilot-Build, der sich selbst aktualisiert (`pilot-release-macos.sh
  --install` oder `Pilot.dmg`).
- **Dev**: `/Applications/Agent-Desk Dev.app` (`fheuer.agentdesk.dev`), gebaut aus dem
  Checkout mit `scripts/install-fork.sh`: eigene Daten
  (`~/Library/Application Support/agent-desk-dev`, `~/.config/agent-desk-dev`), eigenes
  URL-Schema (`agent-desk-dev://`), DEV-Band im Icon, nie Updates.

Startet eine, beendet sie die andere (gleiche Kurzbefehle, gleiches
Mikrofon). `install-fork.sh` entfernt jede andere Kopie (alte
`target/app`-Bundles, Agent-Desk Test, Downloads), damit eine Freigabe in den
Systemeinstellungen immer Live oder Dev gehört; `--dry-run` zeigt die
Liste vorher, `--open` startet Agent-Desk Dev danach.

**Wichtig – Quell-Builds updaten nie.** `scripts/install-fork.sh` (macOS)
und `scripts/install-linux.sh` bauen „lokale Builds“ (`AGENT_DESK_LOCAL_BUILD`,
`source-build`-Marker); die zeigen „Lokaler Build · Updates aus“ und fragen
den Server nie. Um Updates zu bekommen, die veröffentlichte Version
installieren:

- macOS: `scripts/pilot-release-macos.sh … --install` (oder `Pilot.dmg`
  öffnen und `Agent-Desk.app` nach `/Applications` ziehen). Mit demselben
  Signier-Team bleiben die macOS-Berechtigungen erhalten.
- Linux: `curl -fsSL https://ai.sms-pilot.de/install-linux.sh | sh`
- Windows: `PilotSetup.exe` von der Website (oder `install-windows.ps1`).

## Der Release-Schlüssel

- Privat: `~/.config/pilot-release/signing-key` auf diesem Server (Ordner
  0700, Datei 0600). Nie ins Repo, nie auf Mac/PC kopieren: Mac und Windows
  laden nur hoch, signiert wird hier.
- Öffentlich: `src/updater/signature.rs` (`SIGNING_KEY`),
  `.github/release/linux-signing.pub`, `.github/release/install-linux.sh`
  (`signing_key=`). Der Test `the_compiled_key_is_the_published_one` prüft,
  dass alle drei übereinstimmen.
- Signiert wird `pilot.release-artifact.v1\n<sha256>\n` (64 Byte ed25519).
- Schlüssel verloren oder kompromittiert: neuen erzeugen
  (`openssl genpkey -algorithm ed25519`), Public Key an den drei Stellen
  ersetzen, neu bauen. Bereits installierte Apps vertrauen nur dem alten
  Schlüssel – sie müssen die neue Version einmal von Hand installieren.
- **Backup**: Die Datei sicher (offline / Passwortmanager) aufbewahren. Ohne
  sie kann keine installierte App mehr ein Update annehmen.

## Versionen

Pilot-Builds heißen `<Cargo-Version>-pilot.<N>`, z. B. `3.4.0-pilot.1`
(`set-crate-version.sh` stempelt das nur für den Build, nichts wird
committet). Ein Release über mehrere Systeme benutzt **dieselbe Version**:
Linux veröffentlicht zuerst, Mac und Windows fügen mit derselben Version ihre
Dateien hinzu. Eine neue Version beginnt ein neues Release nur mit den
mitgegebenen Dateien – Systeme ohne Datei in dieser Version bleiben, wo sie
sind, und die Website verlinkt für sie weiter die letzte Datei.

Eine Version wird nie mit anderen Bytes erneut veröffentlicht
(`pilot-publish.sh` verweigert das).

## Linux veröffentlichen (hier auf dem Server)

```sh
cd /home/t3/ai-desktop            # sauberer, committeter Stand
export CARGO_TARGET_DIR=/home/t3/ai-desktop/target
scripts/pilot-release-linux.sh                       # nächste -pilot.N
scripts/pilot-release-linux.sh --version 3.4.0-pilot.2 --notes notes.txt
```

Baut nativ (glibc dieses Hosts: Ubuntu 24.04 → braucht glibc 2.39, also
Ubuntu 24.04+, Debian 13, Fedora 40+), mit portablen CPU-Kernen
(`scripts/build-linux.sh`), veröffentlicht mit `pilot-publish.sh` und baut
die Website neu. `.deb`/`.rpm` kommen dazu, sobald `nfpm` installiert ist
(`go install github.com/goreleaser/nfpm/v2/cmd/nfpm@latest`).

## macOS veröffentlichen (auf dem Mac)

Einmalig: SSH-Zugang zu `t3@ai.sms-pilot.de` (Schlüssel in
`~/.ssh/authorized_keys` von `t3`), Xcode mit Zertifikat.

```sh
git pull                                  # derselbe Commit wie das Linux-Release
scripts/pilot-release-macos.sh --version 3.4.0-pilot.1 --install
```

- Mit „Developer ID Application“-Zertifikat und `AGENT_DESK_NOTARY_PROFILE`
  (`xcrun notarytool store-credentials`) wird App und DMG notarisiert – dann
  öffnet jeder Mac sie ohne Warnung.
- Ohne Developer ID nimmt das Skript das „Apple Development“-Zertifikat (wie
  `install-fork.sh`): läuft auf deinen eigenen Macs; andere sehen eine
  Gatekeeper-Warnung (Rechtsklick → Öffnen).
- `--install` ersetzt `/Applications/Agent-Desk.app` (auch einen Quell-Build) durch
  den veröffentlichten Build, der sich ab dann selbst aktualisiert.
- `--no-upload` baut nur (Dateien in `dist/pilot-<version>/`).

## Windows veröffentlichen (auf dem Windows-PC)

Einmalig: Build-Werkzeuge aus `docs/windows.md` (Rust, VS C++ Build Tools,
Vulkan SDK, Git for Windows), OpenSSH-Client mit Schlüssel für
`t3@ai.sms-pilot.de`.

```powershell
git pull
powershell -ExecutionPolicy Bypass -File scripts\pilot-release-windows.ps1 -Version 3.4.0-pilot.1
# mit Code-Signing-Zertifikat:
powershell -ExecutionPolicy Bypass -File scripts\pilot-release-windows.ps1 -Version 3.4.0-pilot.1 -CertificateThumbprint <SHA1>
```

Ohne Zertifikat sind `agent-desk.exe`/`PilotSetup.exe` unsigniert (SmartScreen
warnt beim ersten Start); Updates sind trotzdem durch die Release-Signatur
geschützt. Wer einmal mit Zertifikat veröffentlicht, sollte dabei bleiben:
solche Builds nehmen nur noch Updates mit genau diesem Signierer an.

## Rollback und Status (auf dem Server)

```sh
/srv/pilot/bin/pilot-publish.sh status               # was ist live
/srv/pilot/bin/pilot-publish.sh rollback             # vorherige Version wieder live
```

Rollback veröffentlicht das letzte archivierte Manifest einer anderen
Version mit neuer Sequenznummer; installierte Apps wechseln beim nächsten
Check darauf, die Download-Links folgen. Alte Dateien bleiben in `files/`.
Website zurück: `ln -sfn sites/<älter> /srv/pilot/site.next && mv -T /srv/pilot/site.next /srv/pilot/site`.

## Prüfen von außen

```sh
curl -fsS https://ai.sms-pilot.de/release/hex/desktop/stable.json -o stable.json
url=$(jq -r '.release.targets["linux-x64-sig"].url' stable.json)
curl -fsS "https://ai.sms-pilot.de$url" -o hex.sig
curl -fsSL https://ai.sms-pilot.de/download/linux-x64 -o hex
PILOT_CHECK_MANIFEST=$PWD/stable.json PILOT_CHECK_SIGNATURE=$PWD/hex.sig \
  PILOT_CHECK_FILE=$PWD/hex PILOT_CHECK_TARGET=linux-x64 \
  cargo test --locked --bin agent-desk published_release -- --ignored --nocapture
```

Der Test benutzt den Parser, die Entscheidungsregeln und die
Signaturprüfung der App selbst.

## Was (noch) fehlt

- Impressum/Datenschutz auf der Website (Pflicht für eine öffentliche
  deutsche Seite; Angaben muss der Betreiber liefern).
- Kein Dev-/Beta-Kanal in Benutzung (`--channel beta|dev` geht, Download-
  Links folgen nur `stable`); keine Dev-Notes (`dev-notes.json`).
- Die alten Sparkle-/2.3.1-Feeds von upstream werden hier nicht bedient: es
  gibt keine alten Pilot-Installationen, die sie bräuchten.

## Versions only go up

The release key signs a target's SHA-256, not its version, so a client
takes an `update` or `required` release only when its version is a later
SemVer than its own (`updater::version_is_newer`: `3.4.0-pilot.66` after
`3.4.0-pilot.65`). An old signed build offered again, by a mistake or by
whoever controls the server's files, is ignored. To undo a bad release,
publish the previous code under a new, higher version. Only a channel
switch the person confirmed may go down. Download links (a DMG, release
notes) open only on the update service's own host.
