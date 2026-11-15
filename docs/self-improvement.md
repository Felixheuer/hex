# Self-improvement loop (Agent-Desk improving its own code)

Agent-Desk works on this repository as a background job. Each loop:

1. Think through five real desktop tasks (use cases) and trace them through the code.
2. Write down what is missing, broken, or slow (the to-do list below, newest status).
3. Change the code on a branch, `cargo build`, filtered tests (module path, never the
   audio tests, never several full runs at once), `cargo test --no-run` (integration tests
   include `src/` files by `#[path]`), `cargo clippy` without warnings.
4. PR into `dev` on `Felixheuer/hex` (remote `hex`), merge, `scripts/install-fork.sh`
   (without `--open`).
5. Note the result here so the next loop starts where this one stopped.

## Use cases traced (2026-11-15)

1. "Schreib Michael per iMessage, dass ich 10 Min später komme": contact lookup, messenger
   in the background, proof screenshot.
2. "Trag morgen 15 Uhr Zahnarzt ein und erinnere mich": Calendar / Reminders.
3. "Recherchiere 10 Anbieter, mach eine Excel-Tabelle": parallel browsers, xlsx.
4. "Fasse die PDF in Downloads zusammen und schick sie an Y": file lookup, document read,
   mail attachment.
5. "Was steht in meiner letzten Notiz / such die Mail von Z": Notes, Mail.

## Done (2026-11-15)

| PR | What |
| --- | --- |
| #102 | Onboarding: the Contacts step came back forever. The hardened-runtime app lacked `com.apple.security.personal-information.addressbook`, so macOS never showed the prompt; entitlement added, `validate-app.sh` checks it, `data_access::settle` stops the loop. |
| #103 | `run_osascript` has a 45 s timeout; macOS errors -1743/-600/-1712/-1728/-2741 come with what to do. New app windows are parked at once (place_arrivals before keep_front). |
| #104 | Live pictures: an unchanged app window is captured less often (200 ms → 1 s), any change brings 200 ms back; a connected browser stream keeps a still page "Live". |
| #105 | `hex_lookup` files: folder words (Downloads, Desktop, Documents) narrow the search, kinds (pdf, xlsx, …) too, fresh downloads by date added. Prompt: spreadsheets through `hex_document_write`; the person's own reminders/events go to Reminders/Calendar. |
| #106 | `sync_ext::LockExt::lock_ok` replaces 390 copies of the poison pattern; clippy clean. |
| #107 | A closed app window leaves the job's card (CGWindowList check). |
| #108 | Mail hits carry `mail id N` (Mail's message id) for scripts and attachments; `tests/keyboard_layout.rs` builds again. |
| #109 | No panic on a truncated WAV fmt chunk or a paste index at the text's end. |

## Open (next loop)

- [ ] `hex_calendar_add` / `hex_reminder_add` through EventKit (today: AppleScript to
      Calendar/Reminders works, but is slower and needs Automation consent).
- [ ] Move large inline test modules into `tests.rs` files next to their module.
- [ ] Focus hand-back on macOS 14+ (`ActivateIgnoringOtherApps` is ignored); only testable on
      a real Mac with a person in front.
- [ ] Production `unwrap`/`expect` audit: `cargo clippy -- -W clippy::unwrap_used` runs out of
      memory on this machine; audit module by module instead.
- [ ] Linux/Windows cross-checks can't build here (openssl/blake3 build scripts): leave
      cfg-gated files alone or check them in CI.
