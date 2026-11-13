//! One table per language for each heuristic: what it catches, and what it
//! must leave alone.

use super::*;

/// (language, words, the rest after a spoken opening, or `None`).
const OPENINGS: &[(&str, &str, Option<&str>)] = &[
    (
        "de",
        "Im Hintergrund räum den Downloads-Ordner auf",
        Some("räum den Downloads-Ordner auf"),
    ),
    ("de", "im hintergrund, räum auf.", Some("räum auf.")),
    (
        "de",
        "IM HINTERGRUND: Mails sortieren",
        Some("Mails sortieren"),
    ),
    (
        "de",
        "  „Im Hintergrund“ – buch den Flug",
        Some("buch den Flug"),
    ),
    (
        "de",
        "Nebenbei: schreib Anna zurück",
        Some("schreib Anna zurück"),
    ),
    ("de", "nebenbei bestell Kaffee", Some("bestell Kaffee")),
    (
        "de",
        "parallel lad die Rechnungen herunter",
        Some("lad die Rechnungen herunter"),
    ),
    (
        "de",
        "Hintergrund: Downloads aufräumen",
        Some("Downloads aufräumen"),
    ),
    (
        "de",
        "hintergrund : Downloads aufräumen",
        Some("Downloads aufräumen"),
    ),
    ("de", "Im Hintergrund", Some("")),
    ("de", "Parallelogramm zeichnen", None),
    ("de", "Parallels Desktop öffnen", None),
    ("de", "Hintergrundbild ändern", None),
    ("de", "Hintergrund ändern", None),
    ("de", "Hintergrund-Prozesse anzeigen", None),
    ("de", "Nebenbeigesagt, was steht hier?", None),
    ("de", "Im Hintergrundbild ist was?", None),
    ("de", "Imhintergrund aufräumen", None),
    ("de", "Mach das parallel", None),
    (
        "en",
        "In the background, file these receipts",
        Some("file these receipts"),
    ),
    (
        "en",
        "in the Background: check my calendar",
        Some("check my calendar"),
    ),
    (
        "en",
        "In the background... summarise the PDF",
        Some("summarise the PDF"),
    ),
    (
        "en",
        "Parallel, update the spreadsheet",
        Some("update the spreadsheet"),
    ),
    ("en", "In the background.", Some("")),
    ("en", "In the backgrounds folder, what's new?", None),
    ("en", "In the back, what's that?", None),
    ("en", "In thebackground", None),
    ("en", "What runs in the background?", None),
    (
        "fr",
        "En arrière-plan, range mes téléchargements",
        Some("range mes téléchargements"),
    ),
    ("fr", "Qu'est-ce qui tourne en arrière-plan ?", None),
    (
        "es",
        "En segundo plano: ordena mis descargas",
        Some("ordena mis descargas"),
    ),
    ("es", "¿Qué corre en segundo plano?", None),
    (
        "it",
        "In background, riordina i download",
        Some("riordina i download"),
    ),
    ("it", "Cosa gira in background?", None),
    (
        "nl",
        "Op de achtergrond: ruim mijn downloads op",
        Some("ruim mijn downloads op"),
    ),
    ("nl", "Wat draait er op de achtergrond?", None),
    ("any", "", None),
    ("any", "   ", None),
];

#[test]
fn spoken_openings_route_to_the_background_in_every_language() {
    for &(language, said, rest) in OPENINGS {
        assert_eq!(spoken_background(said), rest, "{language}: {said:?}");
    }
}

/// (language, request, whether it asks to stay in the background).
const BACKGROUND_ONLY_TABLE: &[(&str, &str, bool)] = &[
    (
        "de",
        "Recherchiere das, aber arbeite nur im Hintergrund",
        true,
    ),
    (
        "de",
        "Lass meine Fenster in Ruhe und sortier die Downloads",
        true,
    ),
    ("de", "Übernimm das Citrix-Fenster", false),
    ("de", "Was läuft im Hintergrund?", false),
    ("en", "Do it only in the background", true),
    ("en", "Don’t touch my windows while you do it", true),
    ("en", "Take over the Citrix window", false),
    ("fr", "Fais-le seulement en arrière-plan", true),
    ("fr", "Ouvre la fenêtre Citrix", false),
    ("es", "Hazlo solo en segundo plano", true),
    ("es", "Abre la ventana de Citrix", false),
    ("it", "Fallo solo in background", true),
    ("it", "Apri la finestra di Citrix", false),
    ("nl", "Doe het alleen op de achtergrond", true),
    ("nl", "Open het Citrix-venster", false),
];

#[test]
fn asking_to_stay_in_the_background_in_every_language() {
    for &(language, request, only) in BACKGROUND_ONLY_TABLE {
        assert_eq!(
            asks_background_only(request),
            only,
            "{language}: {request:?}"
        );
    }
}

/// (language, answer, whether it claims to start background work).
const CLAIMS_TABLE: &[(&str, &str, bool)] = &[
    (
        "de",
        "Das läuft als Hintergrundauftrag, und das Video landet im Arbeitsordner.",
        true,
    ),
    ("de", "Mach ich im Hintergrund.", true),
    ("de", "Es ist 13:29 Uhr.", false),
    // A status answer about a job already running starts nothing.
    ("de", "Der Shop-Auftrag läuft noch im Hintergrund.", false),
    ("de", "Das kann ich nicht im Hintergrund erledigen.", false),
    ("en", "I’ll do that in the background.", true),
    ("en", "Two jobs are running in the background.", false),
    ("en", "I can't do that as a background job.", false),
    ("fr", "D'accord, je m'en occupe en arrière-plan.", true),
    ("fr", "Deux tâches tournent en arrière-plan.", false),
    ("es", "Vale, lo hago en segundo plano.", true),
    ("es", "No puedo, lo hago en segundo plano no.", false),
    ("it", "Va bene, lo faccio in background.", true),
    ("it", "Due lavori girano in background.", false),
    ("nl", "Prima, ik doe het op de achtergrond.", true),
    ("nl", "Er draaien twee taken op de achtergrond.", false),
];

#[test]
fn an_answer_that_claims_the_background_is_caught_in_every_language() {
    for &(language, answer, claims) in CLAIMS_TABLE {
        assert_eq!(claims_background(answer), claims, "{language}: {answer:?}");
    }
}

/// (language, words so far, whether they ask for the web).
const WEB_TABLE: &[(&str, &str, bool)] = &[
    ("de", "Schau auf spiegel punkt de", true),
    ("de", "Öffne github.com/anomalyco", true),
    ("de", "such mal auf google nach", true),
    ("de", "Was steht in dieser Mail?", false),
    // No brand words: a name alone says nothing (the sign-in store decides
    // that, and tests never read it); "Punkt" in a sentence isn't a dot.
    ("de", "Öffne WhatsApp", false),
    ("de", "Ein Punkt und noch einer", false),
    ("de", "Version 1.2 ist fertig, z.b. heute.", false),
    ("en", "go to https://example.org", true),
    ("en", "example dot com please", true),
    ("en", "look up the opening hours", true),
    ("en", "What's in this mail?", false),
    ("fr", "Ouvre le navigateur sur lemonde.fr", true),
    ("fr", "Cherche sur internet les horaires", true),
    ("fr", "Qu'est-ce qu'il y a dans ce mail ?", false),
    ("es", "Abre el navegador", true),
    ("es", "Busca en internet el horario", true),
    ("es", "¿Qué dice este correo?", false),
    ("it", "Cerca su internet gli orari", true),
    ("it", "Cosa dice questa mail?", false),
    ("nl", "Zoek op internet de openingstijden", true),
    ("nl", "Wat staat er in deze mail?", false),
];

#[test]
fn words_that_ask_for_the_web_in_every_language() {
    for &(language, words, web) in WEB_TABLE {
        assert_eq!(wants_web(words), web, "{language}: {words:?}");
    }
}
