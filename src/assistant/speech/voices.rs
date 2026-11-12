//! Which installed voice reads an answer: the voices and their quality, the
//! person's choice resolved against them, the best one for a language, and
//! the download that would sound better.

/// How good a voice sounds, worst first.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Quality {
    /// The small voice macOS ships with.
    #[default]
    Default,
    /// A larger download ("Enhanced").
    Enhanced,
    /// The largest, most natural download ("Premium").
    Premium,
    /// The person's own Personal Voice, which Hex may use (macOS).
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only macOS has Personal Voices")
    )]
    Personal,
}

impl Quality {
    /// What Settings shows after a voice's name, if anything.
    fn label(self) -> Option<&'static str> {
        match self {
            Quality::Default => None,
            Quality::Enhanced => Some("Enhanced"),
            Quality::Premium => Some("Premium"),
            Quality::Personal => Some("Personal Voice"),
        }
    }
}

/// An installed voice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Voice {
    /// What Settings shows and saves: unique among the installed voices
    /// (`Anna`, `Petra (Premium)`, `Eddy (de-DE)`).
    pub name: String,
    /// `de_DE`, `en_US`, …
    pub locale: String,
    pub quality: Quality,
    /// What the speech engine knows it by (an AVSpeechSynthesisVoice
    /// identifier, or the `say` name).
    pub id: String,
}

impl Voice {
    /// Its language: `de` for `de_DE`.
    pub fn language(&self) -> &str {
        self.locale.split(['_', '-']).next().unwrap_or_default()
    }

    /// Its region: `DE` for `de_DE`, if it has one.
    fn region(&self) -> Option<&str> {
        self.locale.split(['_', '-']).nth(1)
    }

    /// Its name without the parenthesised quality or language
    /// (`Eddy` for `Eddy (Deutsch (Deutschland))`).
    pub fn base_name(&self) -> &str {
        base_name(&self.name)
    }
}

fn base_name(name: &str) -> &str {
    name.split(" (").next().unwrap_or(name).trim()
}

/// The quality a voice's name says it has (`Zuzana (Premium)`), for engines
/// that don't say otherwise.
pub fn quality_in_name(name: &str) -> Quality {
    let lower = name.to_lowercase();
    if lower.contains("(premium") {
        Quality::Premium
    } else if lower.contains("(enhanced") || lower.contains("(erweitert") {
        Quality::Enhanced
    } else {
        Quality::Default
    }
}

/// A voice as an engine lists it, before names are made unique.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    /// `de-DE` or `de_DE`.
    pub locale: String,
    pub quality: Quality,
}

/// Gives each listed voice a name of its own: its base name, its quality
/// when better than the default, and its locale when the same name speaks
/// more than one language (macOS has an "Eddy" for each).
pub fn named(listed: Vec<Listed>) -> Vec<Voice> {
    let shown = |voice: &Listed| match voice.quality.label() {
        Some(label) => format!("{} ({label})", base_name(&voice.name)),
        None => base_name(&voice.name).to_owned(),
    };
    let names: Vec<String> = listed.iter().map(shown).collect();
    listed
        .into_iter()
        .zip(&names)
        .map(|(voice, name)| {
            let locale = voice.locale.replace('-', "_");
            let shared = names.iter().filter(|other| *other == name).count() > 1;
            let name = if shared {
                format!("{} ({})", base_name(name), voice.locale.replace('_', "-"))
                    + &voice
                        .quality
                        .label()
                        .map(|label| format!(" ({label})"))
                        .unwrap_or_default()
            } else {
                name.clone()
            };
            Voice {
                name,
                locale,
                quality: voice.quality,
                id: voice.id,
            }
        })
        .collect()
}

/// `say -v '?'`'s lines: `Anna                de_DE    # Hallo! Ich heiße Anna.`,
/// where a name can hold spaces and parentheses
/// (`Eddy (Deutsch (Deutschland)) de_DE    # …`). Names stay as `say` knows
/// them, so they work with `say -v`.
#[cfg(any(test, target_os = "macos"))]
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
                quality: quality_in_name(name),
                id: name.to_owned(),
            })
        })
        .collect()
}

/// Novelty voices that read a sentence as a joke, and the oldest robotic
/// ones.
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

/// The Eloquence voices: clear but synthetic, so behind any other voice of
/// the same quality.
const ELOQUENCE: [&str; 8] = [
    "Eddy", "Flo", "Grandma", "Grandpa", "Reed", "Rocko", "Sandy", "Shelley",
];

/// Voices that read well, best first, for the languages answers come in.
const PREFERRED: [(&str, &[&str]); 6] = [
    (
        "de",
        &["Petra", "Anna", "Yannick", "Markus", "Helena", "Martin"],
    ),
    (
        "en",
        &[
            "Ava", "Zoe", "Samantha", "Tom", "Evan", "Susan", "Allison", "Alex", "Daniel", "Karen",
            "Moira",
        ],
    ),
    ("fr", &["Audrey", "Amélie", "Thomas", "Aurelie", "Marie"]),
    (
        "es",
        &["Mónica", "Monica", "Marisol", "Jorge", "Paulina", "Juan"],
    ),
    ("it", &["Federica", "Alice", "Luca", "Paola"]),
    ("nl", &["Claire", "Xander", "Ellen"]),
];

/// The region a language is read in when the system's own doesn't say.
const HOME_REGION: [(&str, &str); 7] = [
    ("de", "DE"),
    ("en", "US"),
    ("fr", "FR"),
    ("es", "ES"),
    ("it", "IT"),
    ("nl", "NL"),
    ("pt", "BR"),
];

fn is_novelty(voice: &Voice) -> bool {
    NOVELTY.contains(&voice.base_name())
        || voice.id.starts_with("com.apple.speech.synthesis.voice.")
            && !matches!(voice.base_name(), "Alex" | "Kathy" | "Vicki" | "Victoria")
}

/// The best installed voice for text in `language`, with `locale` (the
/// system's, `de-DE`) saying which region to prefer: the person's Personal
/// Voice, then Premium, then Enhanced voices, natural ones before
/// synthetic ones, the region's own before others, and known good voices
/// first. Joke voices never read an answer.
pub fn best_voice<'a>(language: &str, locale: &str, voices: &'a [Voice]) -> Option<&'a Voice> {
    let language = language.to_ascii_lowercase();
    let (system_language, system_region) = split_locale(locale);
    let region = if system_language == language && !system_region.is_empty() {
        system_region
    } else {
        HOME_REGION
            .iter()
            .find(|(code, _)| *code == language)
            .map_or("", |(_, region)| *region)
    };
    let preferred = PREFERRED
        .iter()
        .find(|(code, _)| *code == language)
        .map_or(&[][..], |(_, names)| *names);
    voices
        .iter()
        .filter(|voice| voice.language().eq_ignore_ascii_case(&language))
        .filter(|voice| !is_novelty(voice))
        .min_by_key(|voice| {
            let synthetic =
                ELOQUENCE.contains(&voice.base_name()) || voice.id.contains(".eloquence.");
            let other_region = !voice
                .region()
                .is_some_and(|own| own.eq_ignore_ascii_case(region));
            let rank = preferred
                .iter()
                .position(|name| *name == voice.base_name())
                .unwrap_or(preferred.len());
            (
                std::cmp::Reverse(voice.quality),
                synthetic,
                other_region,
                rank,
                voice.name.clone(),
            )
        })
}

fn split_locale(locale: &str) -> (String, &str) {
    let mut parts = locale.split(['_', '-', '.']);
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    (language, parts.next().unwrap_or_default())
}

/// The installed voice the person chose by `chosen` (a name from Settings,
/// a name `say` gave an older Hex, or an engine id), reading text in
/// `language` when known. Several voices share a base name ("Eddy" speaks
/// German and English): the one of the text's language wins, then the
/// better one.
pub fn resolve<'a>(chosen: &str, language: Option<&str>, voices: &'a [Voice]) -> Option<&'a Voice> {
    let chosen = chosen.trim();
    if chosen.is_empty() {
        return None;
    }
    if let Some(voice) = voices
        .iter()
        .find(|voice| voice.id == chosen || voice.name == chosen)
    {
        return Some(voice);
    }
    let base = base_name(chosen);
    let wanted = quality_in_name(chosen);
    voices
        .iter()
        .filter(|voice| voice.base_name().eq_ignore_ascii_case(base))
        .min_by_key(|voice| {
            let other_language =
                !language.is_some_and(|language| voice.language().eq_ignore_ascii_case(language));
            (
                other_language,
                voice.quality != wanted,
                std::cmp::Reverse(voice.quality),
            )
        })
}

/// A voice worth getting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceHint {
    /// As System Settings › Accessibility › Spoken Content lists it
    /// (`Petra (Premium)`); on Windows and Linux, what to get
    /// (`a Piper voice`).
    pub name: String,
    /// `de`, `en`, …
    pub language: String,
    /// About how much it downloads (0 when unknown).
    pub size_mb: u32,
    /// What to tell the person, in English for `tr()`: why, and where to
    /// get it.
    pub advice: &'static str,
}

/// The OS whose voices are hinted at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum System {
    Mac,
    Windows,
    Linux,
}

impl System {
    /// The OS Hex runs on.
    pub const THIS: System = if cfg!(target_os = "windows") {
        System::Windows
    } else if cfg!(target_os = "linux") {
        System::Linux
    } else {
        System::Mac
    };
}

/// The advice for each case, as the UI shows it.
pub const MAC_ADVICE: &str =
    "Premium voices sound far more natural. Download one in System Settings";
pub const WINDOWS_ADVICE: &str =
    "No voice reads this language yet. Add one in Settings › Time & language › Speech";
pub const LINUX_MISSING_ADVICE: &str =
    "No voice reads this language yet. Install Piper with a voice for it, or espeak-ng";
pub const LINUX_PIPER_ADVICE: &str =
    "Piper voices sound far more natural. Install Piper with a voice for this language";

/// The voice to download for each language, its quality, and its download
/// size, from macOS 26's voice catalogue
/// (`com_apple_MobileAsset_VoiceServices_CombinedVocalizerVoices`).
const RECOMMENDED: [(&str, &str, Quality, u32); 16] = [
    ("de", "Petra", Quality::Premium, 487),
    ("en", "Ava", Quality::Premium, 479),
    ("fr", "Audrey", Quality::Premium, 583),
    ("es", "Jorge", Quality::Enhanced, 77),
    ("it", "Federica", Quality::Premium, 180),
    ("nl", "Claire", Quality::Premium, 745),
    ("pt", "Catarina", Quality::Enhanced, 155),
    ("sv", "Oskar", Quality::Premium, 385),
    ("da", "Magnus", Quality::Enhanced, 97),
    ("nb", "Henrik", Quality::Enhanced, 129),
    ("pl", "Ewa", Quality::Enhanced, 128),
    ("cs", "Iveta", Quality::Enhanced, 121),
    ("ru", "Katya", Quality::Premium, 299),
    ("tr", "Cem", Quality::Enhanced, 126),
    ("ja", "Otoya", Quality::Premium, 288),
    ("hi", "Neel", Quality::Premium, 841),
];

/// The voice that would read `language` better than every installed voice
/// of it on `system`, if there is one: on macOS the Premium (or Enhanced)
/// download; on Windows a voice when none reads the language; on Linux a
/// Piper voice when only espeak-ng's (or none) does.
pub fn better_voice_for(system: System, language: &str, voices: &[Voice]) -> Option<VoiceHint> {
    let language = split_locale(language).0;
    let installed = voices
        .iter()
        .filter(|voice| voice.language().eq_ignore_ascii_case(&language))
        .map(|voice| voice.quality)
        .max();
    let hint = |name: String, size_mb, advice| VoiceHint {
        name,
        language: language.clone(),
        size_mb,
        advice,
    };
    match system {
        System::Mac => {
            let (_, name, quality, size_mb) =
                RECOMMENDED.iter().find(|(code, ..)| *code == language)?;
            (installed.unwrap_or_default() < *quality).then(|| {
                hint(
                    format!("{name} ({})", quality.label().unwrap_or_default()),
                    *size_mb,
                    MAC_ADVICE,
                )
            })
        }
        System::Windows => installed
            .is_none()
            .then(|| hint("a Windows voice".into(), 0, WINDOWS_ADVICE)),
        System::Linux => match installed {
            None => Some(hint("a Piper voice".into(), 60, LINUX_MISSING_ADVICE)),
            Some(Quality::Default) => Some(hint("a Piper voice".into(), 60, LINUX_PIPER_ADVICE)),
            Some(_) => None,
        },
    }
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

    fn listed(id: &str, name: &str, locale: &str, quality: Quality) -> Listed {
        Listed {
            id: id.into(),
            name: name.into(),
            locale: locale.into(),
            quality,
        }
    }

    /// What AVSpeechSynthesisVoice lists on a Mac with a few downloads.
    fn engine_voices() -> Vec<Voice> {
        named(vec![
            listed(
                "com.apple.voice.super-compact.de-DE.Anna",
                "Anna",
                "de-DE",
                Quality::Default,
            ),
            listed(
                "com.apple.eloquence.de-DE.Eddy",
                "Eddy",
                "de-DE",
                Quality::Default,
            ),
            listed(
                "com.apple.eloquence.en-US.Eddy",
                "Eddy",
                "en-US",
                Quality::Default,
            ),
            listed(
                "com.apple.voice.premium.de-DE.Petra",
                "Petra (Premium)",
                "de-DE",
                Quality::Premium,
            ),
            listed(
                "com.apple.voice.enhanced.en-GB.Serena",
                "Serena",
                "en-GB",
                Quality::Enhanced,
            ),
            listed(
                "com.apple.voice.compact.en-US.Samantha",
                "Samantha",
                "en-US",
                Quality::Default,
            ),
            listed(
                "com.apple.speech.synthesis.voice.Bubbles",
                "Bubbles",
                "en-US",
                Quality::Default,
            ),
        ])
    }

    #[test]
    fn say_voices_keep_names_with_spaces_and_parentheses() {
        let voices = parse_voices(SAY_LIST);
        assert_eq!(voices.len(), 7);
        assert_eq!(voices[2].name, "Eddy (Deutsch (Deutschland))");
        assert_eq!(voices[2].id, "Eddy (Deutsch (Deutschland))");
        assert_eq!(voices[2].locale, "de_DE");
        assert_eq!(voices[2].base_name(), "Eddy");
        assert_eq!(voices[5].name, "Zuzana (Premium)");
        assert_eq!(voices[5].quality, Quality::Premium);
        assert_eq!(voices[5].language(), "cs");
        assert!(parse_voices("\n   \nnonsense\n").is_empty());
    }

    #[test]
    fn engine_voices_get_unique_names_that_tell_their_quality() {
        let voices = engine_voices();
        let names: Vec<&str> = voices.iter().map(|voice| voice.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Anna",
                "Eddy (de-DE)",
                "Eddy (en-US)",
                "Petra (Premium)",
                "Serena (Enhanced)",
                "Samantha",
                "Bubbles"
            ]
        );
        assert_eq!(voices[3].locale, "de_DE");
        let personal = named(vec![listed("p1", "Felix", "de-DE", Quality::Personal)]);
        assert_eq!(personal[0].name, "Felix (Personal Voice)");
    }

    #[test]
    fn the_best_voice_is_the_best_quality_of_the_language() {
        let voices = engine_voices();
        let best = |language, locale| best_voice(language, locale, &voices).map(|v| &*v.name);
        assert_eq!(best("de", "en-US"), Some("Petra (Premium)"));
        // Enhanced from another region beats the default voice of the home
        // region.
        assert_eq!(best("en", "de-DE"), Some("Serena (Enhanced)"));
        assert_eq!(best("cs", "de-DE"), None);
        let defaults: Vec<Voice> = voices
            .iter()
            .filter(|voice| voice.quality == Quality::Default)
            .cloned()
            .collect();
        // Natural before synthetic (Eloquence), and never a joke voice.
        assert_eq!(
            best_voice("de", "de-DE", &defaults).map(|v| &*v.name),
            Some("Anna")
        );
        assert_eq!(
            best_voice("en", "de-DE", &defaults).map(|v| &*v.name),
            Some("Samantha")
        );
        let personal = named(vec![
            listed("premium", "Ava", "en-US", Quality::Premium),
            listed("mine", "Felix", "en-US", Quality::Personal),
        ]);
        assert_eq!(
            best_voice("en", "en-US", &personal).map(|v| &*v.id),
            Some("mine")
        );
    }

    #[test]
    fn the_region_decides_between_equal_voices() {
        let voices = named(vec![
            listed("gb", "Daniel", "en-GB", Quality::Enhanced),
            listed("us", "Zoe", "en-US", Quality::Enhanced),
            listed("au", "Karen", "en-AU", Quality::Enhanced),
        ]);
        let best = |locale| best_voice("en", locale, &voices).map(|v| &*v.id);
        assert_eq!(best("en-GB"), Some("gb"));
        assert_eq!(best("en_AU"), Some("au"));
        // Another system language: the language's home region.
        assert_eq!(best("de-DE"), Some("us"));
    }

    #[test]
    fn novelty_voices_are_never_picked() {
        let voices = parse_voices(
            "Albert              en_US    # Hello!\nBad News            en_US    # Hello!\nFlo (Englisch (USA)) en_US    # Hello!\n",
        );
        assert_eq!(
            best_voice("en", "en_US", &voices).map(|v| &*v.name),
            Some("Flo (Englisch (USA))")
        );
        let only_jokes = parse_voices("Bubbles             en_US    # Hello!\n");
        assert_eq!(best_voice("en", "en_US", &only_jokes), None);
    }

    #[test]
    fn a_chosen_voice_is_found_by_any_name_it_had() {
        let voices = engine_voices();
        let found = |chosen, language| resolve(chosen, language, &voices).map(|v| &*v.id);
        assert_eq!(
            found("Petra (Premium)", None),
            Some("com.apple.voice.premium.de-DE.Petra")
        );
        assert_eq!(
            found("com.apple.eloquence.en-US.Eddy", Some("de")),
            Some("com.apple.eloquence.en-US.Eddy")
        );
        // A name `say` gave: the voice of the text's language.
        assert_eq!(
            found("Eddy (Deutsch (Deutschland))", Some("de")),
            Some("com.apple.eloquence.de-DE.Eddy")
        );
        assert_eq!(
            found("Eddy (Englisch (USA))", Some("en")),
            Some("com.apple.eloquence.en-US.Eddy")
        );
        assert_eq!(
            found("petra", Some("de")),
            Some("com.apple.voice.premium.de-DE.Petra")
        );
        assert_eq!(found("Nobody", Some("de")), None);
        assert_eq!(found("  ", None), None);
        // Settings' names resolve against `say`'s list too, for the fallback.
        let say = parse_voices(SAY_LIST);
        assert_eq!(
            resolve("Eddy (de-DE)", Some("de"), &say).map(|v| &*v.id),
            Some("Eddy (Deutsch (Deutschland))")
        );
    }

    #[test]
    fn a_better_download_is_suggested_until_one_is_installed() {
        let defaults = parse_voices(SAY_LIST);
        assert_eq!(
            better_voice_for(System::Mac, "de", &defaults),
            Some(VoiceHint {
                name: "Petra (Premium)".into(),
                language: "de".into(),
                size_mb: 487,
                advice: MAC_ADVICE,
            })
        );
        assert_eq!(
            better_voice_for(System::Mac, "en-US", &defaults).map(|hint| hint.name),
            Some("Ava (Premium)".into())
        );
        let voices = engine_voices();
        assert_eq!(better_voice_for(System::Mac, "de_DE", &voices), None);
        // Enhanced English is installed; Premium is still better.
        assert_eq!(
            better_voice_for(System::Mac, "en", &voices).map(|hint| hint.size_mb),
            Some(479)
        );
        assert_eq!(better_voice_for(System::Mac, "xx", &voices), None);
        let spanish = named(vec![listed("j", "Jorge", "es-ES", Quality::Enhanced)]);
        assert_eq!(better_voice_for(System::Mac, "es", &spanish), None);
    }

    #[test]
    fn windows_and_linux_hint_where_to_get_a_voice() {
        let installed = named(vec![
            listed("espeak:de", "German", "de", Quality::Default),
            listed("piper:/en.onnx", "Lessac", "en_US", Quality::Enhanced),
        ]);
        let hint = |system, language| better_voice_for(system, language, &installed);
        assert_eq!(hint(System::Windows, "de"), None);
        assert_eq!(
            hint(System::Windows, "fr").map(|hint| hint.advice),
            Some(WINDOWS_ADVICE)
        );
        assert_eq!(
            hint(System::Linux, "de").map(|hint| hint.advice),
            Some(LINUX_PIPER_ADVICE)
        );
        assert_eq!(hint(System::Linux, "en-US"), None);
        assert_eq!(
            hint(System::Linux, "fr").map(|hint| hint.advice),
            Some(LINUX_MISSING_ADVICE)
        );
    }
}
