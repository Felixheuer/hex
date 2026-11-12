//! Reading answers aloud with the macOS voice, on this computer, through
//! `/usr/bin/say`: one process at a time, killed by [`stop`] and by the
//! next [`speak`].
//!
//! Without a chosen voice, text in the system's language uses the system
//! voice; text in another language (a German answer on an English Mac) uses
//! an installed voice of that language, if there is one.
//!
//! OWNER: hands (with `computer`).

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

const SAY: &str = "/usr/bin/say";

static PLAYER: Mutex<Player> = Mutex::new(Player { child: None });

/// Starts reading `text` aloud with `voice`, stopping anything already
/// being read. `None` picks by the answer's own language
/// ([`automatic_voice`]): the system voice for text in the system's
/// language, else an installed voice of the text's language. Returns at
/// once.
pub fn speak(text: &str, voice: Option<&str>) {
    let text = speakable(text);
    let mut player = PLAYER.lock().unwrap_or_else(|error| error.into_inner());
    player.stop();
    if text.is_empty() {
        return;
    }
    let voice = voice.map(str::to_owned).or_else(|| automatic_voice(&text));
    let mut command = crate::desktop::background_process::command(SAY);
    if let Some(voice) = &voice {
        command.args(["-v", voice]);
    }
    // The text goes in on stdin, so one that starts with "-" is never an
    // option.
    command.args(["-f", "-"]);
    tracing::debug!(
        voice = voice.as_deref().unwrap_or("system"),
        "reading aloud"
    );
    player.play(command, text);
}

/// Stops reading aloud: `say` and anything it started are killed.
pub fn stop() {
    PLAYER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stop();
}

/// Whether something is being read now.
pub fn speaking() -> bool {
    PLAYER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .playing()
}

/// One reading-aloud process at a time, in a process group of its own so a
/// stop ends everything it started. The seam the tests drive with a stand-in
/// for `say`.
struct Player {
    child: Option<Child>,
}

impl Player {
    /// Stops what plays, then starts `command`, handing it `text` on stdin.
    fn play(&mut self, mut command: Command, text: String) {
        self.stop();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(%error, "could not start reading aloud");
                return;
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let spawned = std::thread::Builder::new()
                .name("hex-assistant-say".into())
                .spawn(move || {
                    // A stop before it's all written ends the pipe; that's fine.
                    let _ = stdin.write_all(text.as_bytes());
                });
            if let Err(error) = spawned {
                tracing::warn!(%error, "could not hand the text to say");
            }
        }
        self.child = Some(child);
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            crate::assistant::backends::run::kill_group(&mut child);
        }
    }

    fn playing(&mut self) -> bool {
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                // Done: reap it.
                self.child = None;
                false
            }
            None => false,
        }
    }
}

/// The installed voices' names, for Settings.
pub fn voices() -> Vec<String> {
    voices_with_languages()
        .into_iter()
        .map(|voice| voice.name)
        .collect()
}

/// An installed voice and its language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Voice {
    pub name: String,
    /// `de_DE`, `en_US`, …
    pub locale: String,
}

impl Voice {
    /// Its language: `de` for `de_DE`.
    pub fn language(&self) -> &str {
        self.locale.split(['_', '-']).next().unwrap_or_default()
    }
}

/// The installed voices with their languages (`say -v '?'`), read once.
pub fn voices_with_languages() -> Vec<Voice> {
    static VOICES: OnceLock<Vec<Voice>> = OnceLock::new();
    VOICES
        .get_or_init(|| {
            let output = crate::desktop::background_process::command(SAY)
                .args(["-v", "?"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            match output {
                Ok(output) => parse_voices(&String::from_utf8_lossy(&output.stdout)),
                Err(error) => {
                    tracing::warn!(%error, "could not list the voices");
                    Vec::new()
                }
            }
        })
        .clone()
}

/// `say -v '?'`'s lines: `Anna                de_DE    # Hallo! Ich heiße Anna.`,
/// where a name can hold spaces and parentheses
/// (`Eddy (Deutsch (Deutschland)) de_DE    # …`).
pub fn parse_voices(output: &str) -> Vec<Voice> {
    output
        .lines()
        .filter_map(|line| {
            let described = line
                .split_once('#')
                .map_or(line, |(voice, _)| voice)
                .trim_end();
            let (name, locale) = described.rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            let locale = locale.trim();
            let looks_like_locale = locale.len() >= 2
                && locale
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            (!name.is_empty() && looks_like_locale).then(|| Voice {
                name: name.to_owned(),
                locale: locale.to_owned(),
            })
        })
        .collect()
}

/// The system's language (`de` for a German Mac), from AppleLanguages.
fn system_language() -> String {
    static LANGUAGE: OnceLock<String> = OnceLock::new();
    LANGUAGE
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            let preferred = objc2::rc::autoreleasepool(|_| {
                objc2_foundation::NSLocale::preferredLanguages()
                    .firstObject()
                    .map(|language| language.to_string())
            });
            #[cfg(not(target_os = "macos"))]
            let preferred = std::env::var("LANG").ok();
            preferred
                .and_then(|language| language.split(['-', '_', '.']).next().map(str::to_owned))
                .unwrap_or_else(|| "en".into())
                .to_ascii_lowercase()
        })
        .clone()
}

/// The voice [`speak`] reads `text` with when none is chosen: `None` for the
/// system voice, else a voice of the text's language (see [`voice_for`]).
pub fn automatic_voice(text: &str) -> Option<String> {
    voice_for(text, &voices_with_languages(), &system_language())
}

/// The voice for `text` when none is chosen: `None` (the system voice) when
/// the text is in the system's language or its language is unclear, else
/// the best installed voice of its language.
pub fn voice_for(text: &str, voices: &[Voice], system_language: &str) -> Option<String> {
    let language = language_of(text)?;
    if language == system_language {
        return None;
    }
    best_voice(language, voices)
}

/// Novelty voices that read a sentence as a joke.
const NOVELTY: [&str; 18] = [
    "Albert",
    "Bad News",
    "Bahh",
    "Bells",
    "Boing",
    "Bubbles",
    "Cellos",
    "Good News",
    "Jester",
    "Organ",
    "Superstar",
    "Trinoids",
    "Whisper",
    "Wobble",
    "Zarvox",
    "Junior",
    "Ralph",
    "Fred",
];

/// Voices that read well, best first, for the languages answers come in.
const PREFERRED: [(&str, &[&str]); 6] = [
    (
        "de",
        &["Anna", "Petra", "Markus", "Yannick", "Helena", "Martin"],
    ),
    (
        "en",
        &["Samantha", "Alex", "Daniel", "Karen", "Moira", "Ava", "Tom"],
    ),
    ("fr", &["Thomas", "Amélie", "Audrey", "Marie"]),
    ("es", &["Mónica", "Monica", "Jorge", "Paulina", "Juan"]),
    ("it", &["Alice", "Federica", "Luca"]),
    ("nl", &["Xander", "Claire", "Ellen"]),
];

fn best_voice(language: &str, voices: &[Voice]) -> Option<String> {
    let of_language: Vec<&Voice> = voices
        .iter()
        .filter(|voice| voice.language().eq_ignore_ascii_case(language))
        .filter(|voice| {
            !NOVELTY
                .iter()
                .any(|novelty| voice.name.starts_with(novelty))
        })
        .collect();
    let base = |voice: &Voice| {
        voice
            .name
            .split(" (")
            .next()
            .unwrap_or(&voice.name)
            .to_owned()
    };
    let quality = |voice: &Voice| {
        if voice.name.contains("Premium") {
            0
        } else if voice.name.contains("Enhanced") || voice.name.contains("Erweitert") {
            1
        } else {
            2
        }
    };
    let preferred = PREFERRED
        .iter()
        .find(|(code, _)| *code == language)
        .map_or(&[][..], |(_, names)| *names);
    preferred
        .iter()
        .find_map(|name| {
            of_language
                .iter()
                .filter(|voice| base(voice) == *name)
                .min_by_key(|voice| quality(voice))
        })
        .or_else(|| of_language.iter().min_by_key(|voice| quality(voice)))
        .map(|voice| voice.name.clone())
}

/// Common words that tell these languages apart.
const MARKERS: [(&str, &[&str]); 6] = [
    (
        "de",
        &[
            "der", "die", "das", "und", "ist", "nicht", "ich", "du", "sie", "ein", "eine", "mit",
            "auf", "für", "es", "zu", "den", "dem", "wir", "bereit", "auch", "noch",
        ],
    ),
    (
        "en",
        &[
            "the", "and", "is", "are", "you", "it", "of", "to", "in", "that", "this", "with",
            "for", "not", "be", "your", "can",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "et", "est", "vous", "une", "des", "pas", "que", "pour", "avec",
        ],
    ),
    (
        "es",
        &[
            "el", "los", "las", "y", "es", "una", "que", "por", "para", "con", "está", "pero",
        ],
    ),
    (
        "it",
        &[
            "il", "lo", "gli", "e", "è", "una", "che", "per", "non", "sono", "con", "della",
        ],
    ),
    (
        "nl",
        &[
            "de", "het", "een", "en", "is", "niet", "ik", "je", "van", "dat", "met", "voor",
        ],
    ),
];

/// The language `text` is most likely in, by its common words (German also
/// by ä, ö, ü, ß); `None` when too little tells.
pub fn language_of(text: &str) -> Option<&'static str> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut scores: Vec<(&'static str, usize)> = MARKERS
        .iter()
        .map(|(language, markers)| {
            let hits = words
                .iter()
                .filter(|word| markers.contains(&word.as_str()))
                .count();
            (*language, hits)
        })
        .collect();
    let umlauts = text
        .chars()
        .filter(|c| matches!(c, 'ä' | 'ö' | 'ü' | 'ß' | 'Ä' | 'Ö' | 'Ü'))
        .count();
    if let Some(german) = scores.iter_mut().find(|(language, _)| *language == "de") {
        german.1 += umlauts.min(3);
    }
    scores.sort_by_key(|(_, hits)| std::cmp::Reverse(*hits));
    match scores.as_slice() {
        [(language, best), (_, next), ..] if *best >= 1 && best > next => Some(language),
        _ => None,
    }
}

/// `text` as it should be heard: Markdown's marks and code fences dropped.
fn speakable(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .map(|line| {
            line.trim_start_matches(['#', '>', ' '])
                .replace("**", "")
                .replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAY_LIST: &str = "Albert              en_US    # Hello! My name is Albert.
Anna                de_DE    # Hallo! Ich heiße Anna.
Eddy (Deutsch (Deutschland)) de_DE    # Hallo! Ich heiße Eddy.
Eddy (Englisch (USA)) en_US    # Hello! My name is Eddy.
Samantha            en_US    # Hello! My name is Samantha.
Zuzana (Premium)    cs_CZ    # Ahoj! Jmenuji se Zuzana.
Thomas              fr_FR    # Bonjour, je m’appelle Thomas.
";

    #[test]
    fn voices_keep_names_with_spaces_and_parentheses() {
        let voices = parse_voices(SAY_LIST);
        assert_eq!(voices.len(), 7);
        assert_eq!(
            voices[2],
            Voice {
                name: "Eddy (Deutsch (Deutschland))".into(),
                locale: "de_DE".into()
            }
        );
        assert_eq!(voices[5].name, "Zuzana (Premium)");
        assert_eq!(voices[5].language(), "cs");
        assert!(parse_voices("\n   \nnonsense\n").is_empty());
    }

    #[test]
    fn the_language_is_told_by_common_words() {
        assert_eq!(language_of("Hex Assistent ist bereit"), Some("de"));
        assert_eq!(
            language_of("Die Datei ist in deinem Downloads-Ordner."),
            Some("de")
        );
        assert_eq!(
            language_of("The file is in your Downloads folder."),
            Some("en")
        );
        assert_eq!(
            language_of("Le fichier est dans vos téléchargements et pas ici."),
            Some("fr")
        );
        assert_eq!(language_of("Größe: 12 MB"), Some("de"));
        assert_eq!(language_of("OK"), None);
        assert_eq!(language_of(""), None);
    }

    #[test]
    fn text_in_another_language_gets_a_voice_of_its_own() {
        let voices = parse_voices(SAY_LIST);
        assert_eq!(
            voice_for("The file is in your Downloads folder.", &voices, "de").as_deref(),
            Some("Samantha")
        );
        assert_eq!(
            voice_for("Hex Assistent ist bereit", &voices, "en").as_deref(),
            Some("Anna")
        );
        // The system's own language keeps the system voice.
        assert_eq!(voice_for("Hex Assistent ist bereit", &voices, "de"), None);
        // No voice of that language: the system voice.
        assert_eq!(
            voice_for("Il file è nella cartella per te", &voices, "de"),
            None
        );
    }

    #[test]
    fn novelty_voices_are_never_picked() {
        let voices = parse_voices(
            "Albert              en_US    # Hello!\nBad News            en_US    # Hello!\nFlo (Englisch (USA)) en_US    # Hello!\n",
        );
        assert_eq!(
            best_voice("en", &voices).as_deref(),
            Some("Flo (Englisch (USA))")
        );
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: Signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn gone_soon(pid: i32) -> bool {
        let started = std::time::Instant::now();
        while alive(pid) && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        !alive(pid)
    }

    /// A stand-in for `say` that reads its text and starts a child of its
    /// own, telling us both pids.
    fn stand_in(pids: &std::path::Path) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            &format!(
                "sleep 60 & echo $$ $! > '{}'; cat > /dev/null; wait",
                pids.display()
            ),
        ]);
        command
    }

    fn pids(file: &std::path::Path) -> (i32, i32) {
        let started = std::time::Instant::now();
        loop {
            if let Ok(text) = std::fs::read_to_string(file)
                && let [parent, child] = text.split_whitespace().collect::<Vec<_>>()[..]
            {
                return (parent.parse().unwrap(), child.parse().unwrap());
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn stop_kills_the_reader_and_everything_it_started() {
        let folder = tempfile_dir("stop");
        let file = folder.join("pids");
        let mut player = Player { child: None };
        player.play(stand_in(&file), "Hallo".into());
        let (parent, child) = pids(&file);
        assert!(player.playing());
        player.stop();
        assert!(!player.playing());
        assert!(gone_soon(parent), "the reader still runs");
        assert!(gone_soon(child), "what it started still runs");
    }

    #[test]
    fn a_new_reading_ends_the_one_before() {
        let folder = tempfile_dir("replace");
        let (first, second) = (folder.join("first"), folder.join("second"));
        let mut player = Player { child: None };
        player.play(stand_in(&first), "Eins".into());
        let (parent, child) = pids(&first);
        player.play(stand_in(&second), "Zwei".into());
        assert!(gone_soon(parent) && gone_soon(child));
        let (next, _) = pids(&second);
        assert!(alive(next) && player.playing());
        player.stop();
    }

    fn tempfile_dir(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "hex-speech-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn markdown_is_not_read_out() {
        assert_eq!(
            speakable("## Result\n**Done**, see `notes.txt`.\n```sh\nls\n```"),
            "Result\nDone, see notes.txt.\nls"
        );
    }
}
