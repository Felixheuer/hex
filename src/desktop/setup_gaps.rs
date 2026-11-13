//! What the assistant's setup is missing now, after it was finished once: a
//! permission an update or macOS took away, an agent that was removed or
//! signed out, a shortcut another app took, a voice whose files are gone,
//! or a setup step a release added. Agent-Desk checks at launch and every few
//! minutes; when something is missing it brings back only the onboarding
//! pages that fix it (`rune_ui::chats::setup::repair`), and a hold of the
//! Action shortcut says what's missing.
//!
//! Pure: [`Facts`] are read elsewhere (`rune_ui::chats::jobs`, off the UI
//! thread), so every rule here is a unit test.

/// Bumped whenever a release adds a setup step (a page in [`ADDED`]).
/// Version 1 is the assistant setup as it shipped; anyone who finished it
/// is at 1.
pub const SETUP_VERSION: u32 = 1;

/// Setup pages a release added, with the version that added them, by the
/// page's name (`rune_ui::chats::setup::Page::name`). Someone who finished
/// setup at an older version sees just these.
pub const ADDED: &[(u32, &str)] = &[];

/// What Agent-Desk knows about this Mac, read afresh.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Something keeps macOS from giving Agent-Desk permissions (a translocated
    /// or stray copy; `permission_doctor`).
    pub copy_problem: bool,
    pub microphone: bool,
    pub accessibility: bool,
    pub screen_recording: bool,
    /// Agent-Desk may control other apps (Automation for System Events and
    /// Finder), or macOS couldn't tell.
    pub automation: bool,
    /// macOS asked about Contacts, Reminders, and the Calendar, and the
    /// person answered each (yes or no: `desktop::data_access`).
    pub personal_data: bool,
    /// The selected transcription model is on disk.
    pub transcription_model: bool,
    /// The chosen agent is installed.
    pub agent_installed: bool,
    /// The chosen agent is installed and, as far as Agent-Desk can tell, signed in.
    pub agent_ready: bool,
    /// Tasks operate apps (`assistant.control_desktop`).
    pub desktop_wanted: bool,
    pub agent_desktop_installed: bool,
    /// agent-desktop has Accessibility and Screen Recording.
    pub agent_desktop_grants: bool,
    /// Web tasks use hidden browsers (`assistant.background_browser`).
    pub browser_wanted: bool,
    pub agent_browser_installed: bool,
    /// Another app or macOS feature takes the Action shortcut.
    pub shortcut_taken: bool,
    /// An Agent-Desk voice (`hex-voice:*`) is selected …
    pub hex_voice_selected: bool,
    /// … and its files are there.
    pub hex_voice_installed: bool,
    /// The setup version the person finished (`assistant.setup_version`).
    pub setup_version: u32,
}

/// One missing thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gap {
    /// macOS can't give this copy of Agent-Desk permissions.
    CopyProblem,
    Microphone,
    Accessibility,
    ScreenRecording,
    /// Automation for System Events: tasks can't script other apps without
    /// a prompt in the middle of their work.
    Automation,
    /// Contacts, Reminders, or the Calendar was never asked for: the
    /// assistant can't look someone up or read the agenda.
    PersonalData,
    TranscriptionModel,
    /// The chosen agent isn't installed any more.
    Agent,
    /// It's installed but not signed in.
    AgentSignIn,
    AgentDesktop,
    AgentDesktopGrants,
    AgentBrowser,
    Shortcut,
    /// The selected Agent-Desk voice's files are gone.
    HexVoice,
    /// Steps a release added since the person's setup, by page name.
    NewSteps(Vec<&'static str>),
}

/// How a gap is fixed: on an onboarding page, or with one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// The onboarding page with this name.
    Page(&'static str),
    /// Downloads the selected Agent-Desk voice again.
    DownloadVoice,
    /// Opens the transcription models in Settings.
    Models,
}

impl Gap {
    /// Where it's fixed. `NewSteps` is fixed on each of its pages; this
    /// names the first.
    pub fn fix(&self) -> Fix {
        match self {
            Self::CopyProblem => Fix::Page("checks"),
            Self::Microphone
            | Self::Accessibility
            | Self::ScreenRecording
            | Self::Automation
            | Self::PersonalData
            | Self::AgentDesktop
            | Self::AgentDesktopGrants => Fix::Page("desktop"),
            Self::TranscriptionModel => Fix::Models,
            Self::Agent | Self::AgentSignIn => Fix::Page("agent"),
            Self::AgentBrowser => Fix::Page("browser"),
            Self::Shortcut => Fix::Page("shortcut"),
            Self::HexVoice => Fix::DownloadVoice,
            Self::NewSteps(pages) => Fix::Page(pages.first().copied().unwrap_or("welcome")),
        }
    }

    /// Whether the assistant can't take a question without it: a hold then
    /// says what's missing instead of asking.
    pub fn essential(&self) -> bool {
        matches!(
            self,
            Self::CopyProblem
                | Self::Microphone
                | Self::Accessibility
                | Self::Agent
                | Self::AgentSignIn
        )
    }

    /// What's missing, plainly, as the guide's header and the HUD say it
    /// (English; translated where shown).
    pub fn message(&self) -> &'static str {
        match self {
            Self::CopyProblem => "macOS can't give this copy of Agent-Desk its permissions",
            Self::Microphone => "Agent-Desk can't hear you: the microphone permission is off",
            Self::Accessibility => {
                "Agent-Desk can't see its shortcut or read text: Accessibility is off"
            }
            Self::ScreenRecording => "Agent-Desk can't see the screen: Screen Recording is off",
            Self::Automation => "Agent-Desk may not control other apps yet: Automation is off",
            Self::PersonalData => {
                "New: let the assistant read your contacts, reminders, and calendar"
            }
            Self::TranscriptionModel => {
                "The speech model is missing, so Agent-Desk can't understand you"
            }
            Self::Agent => "The assistant's agent isn't installed any more",
            Self::AgentSignIn => "The assistant's agent is signed out",
            Self::AgentDesktop => "agent-desktop, which operates apps, is missing",
            Self::AgentDesktopGrants => {
                "agent-desktop lost its permissions, so tasks can't use apps"
            }
            Self::AgentBrowser => "agent-browser, for web tasks, is missing",
            Self::Shortcut => "Another app took the assistant's shortcut",
            Self::HexVoice => "The Agent-Desk voice's files are missing",
            Self::NewSteps(_) => "New in this update: a short setup step",
        }
    }
}

/// Everything missing, most essential first. Empty when setup is whole.
pub fn setup_gaps(facts: &Facts) -> Vec<Gap> {
    let mut gaps = Vec::new();
    if facts.copy_problem {
        gaps.push(Gap::CopyProblem);
    }
    if !facts.microphone {
        gaps.push(Gap::Microphone);
    }
    if !facts.accessibility {
        gaps.push(Gap::Accessibility);
    }
    if !facts.agent_installed {
        gaps.push(Gap::Agent);
    } else if !facts.agent_ready {
        gaps.push(Gap::AgentSignIn);
    }
    if !facts.transcription_model {
        gaps.push(Gap::TranscriptionModel);
    }
    if !facts.screen_recording {
        gaps.push(Gap::ScreenRecording);
    }
    if facts.desktop_wanted && !facts.automation {
        gaps.push(Gap::Automation);
    }
    if !facts.personal_data {
        gaps.push(Gap::PersonalData);
    }
    if facts.desktop_wanted {
        if !facts.agent_desktop_installed {
            gaps.push(Gap::AgentDesktop);
        } else if !facts.agent_desktop_grants {
            gaps.push(Gap::AgentDesktopGrants);
        }
    }
    if facts.browser_wanted && !facts.agent_browser_installed {
        gaps.push(Gap::AgentBrowser);
    }
    if facts.shortcut_taken {
        gaps.push(Gap::Shortcut);
    }
    if facts.hex_voice_selected && !facts.hex_voice_installed {
        gaps.push(Gap::HexVoice);
    }
    let added = added_since(facts.setup_version);
    if !added.is_empty() {
        gaps.push(Gap::NewSteps(added));
    }
    gaps
}

/// The pages [`ADDED`] after `version`, in setup order of addition.
pub fn added_since(version: u32) -> Vec<&'static str> {
    added_since_in(ADDED, version)
}

fn added_since_in(added: &[(u32, &'static str)], version: u32) -> Vec<&'static str> {
    let mut pages: Vec<&'static str> = Vec::new();
    for (since, page) in added {
        if *since > version && !pages.contains(page) {
            pages.push(page);
        }
    }
    pages
}

/// The onboarding pages that fix `gaps`, each once, in the order `order`
/// lists pages (the flow's own order).
pub fn pages_for(gaps: &[Gap], order: &[&'static str]) -> Vec<&'static str> {
    let mut wanted: Vec<&'static str> = Vec::new();
    for gap in gaps {
        match gap {
            Gap::NewSteps(pages) => wanted.extend(pages.iter().copied()),
            other => {
                if let Fix::Page(page) = other.fix() {
                    wanted.push(page);
                }
            }
        }
    }
    order
        .iter()
        .copied()
        .filter(|page| wanted.contains(page))
        .collect()
}

/// The gap a hold of the Action shortcut says instead of asking, if any.
pub fn essential(gaps: &[Gap]) -> Option<&Gap> {
    gaps.iter().find(|gap| gap.essential())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole() -> Facts {
        Facts {
            copy_problem: false,
            microphone: true,
            accessibility: true,
            screen_recording: true,
            automation: true,
            personal_data: true,
            transcription_model: true,
            agent_installed: true,
            agent_ready: true,
            desktop_wanted: true,
            agent_desktop_installed: true,
            agent_desktop_grants: true,
            browser_wanted: true,
            agent_browser_installed: true,
            shortcut_taken: false,
            hex_voice_selected: true,
            hex_voice_installed: true,
            setup_version: SETUP_VERSION,
        }
    }

    const ORDER: &[&str] = &[
        "welcome", "privacy", "agent", "desktop", "browser", "checks", "shortcut", "about", "try",
    ];

    #[test]
    fn a_whole_setup_has_no_gaps() {
        assert!(setup_gaps(&whole()).is_empty());
    }

    #[test]
    fn each_missing_thing_is_one_gap_on_its_page() {
        type Case = (fn(&mut Facts), Gap, Fix);
        let cases: Vec<Case> = vec![
            (
                |f| f.copy_problem = true,
                Gap::CopyProblem,
                Fix::Page("checks"),
            ),
            (
                |f| f.microphone = false,
                Gap::Microphone,
                Fix::Page("desktop"),
            ),
            (
                |f| f.accessibility = false,
                Gap::Accessibility,
                Fix::Page("desktop"),
            ),
            (
                |f| f.screen_recording = false,
                Gap::ScreenRecording,
                Fix::Page("desktop"),
            ),
            (
                |f| f.transcription_model = false,
                Gap::TranscriptionModel,
                Fix::Models,
            ),
            (
                |f| f.agent_installed = false,
                Gap::Agent,
                Fix::Page("agent"),
            ),
            (
                |f| f.agent_ready = false,
                Gap::AgentSignIn,
                Fix::Page("agent"),
            ),
            (
                |f| f.agent_desktop_installed = false,
                Gap::AgentDesktop,
                Fix::Page("desktop"),
            ),
            (
                |f| f.agent_desktop_grants = false,
                Gap::AgentDesktopGrants,
                Fix::Page("desktop"),
            ),
            (
                |f| f.agent_browser_installed = false,
                Gap::AgentBrowser,
                Fix::Page("browser"),
            ),
            (
                |f| f.shortcut_taken = true,
                Gap::Shortcut,
                Fix::Page("shortcut"),
            ),
            (
                |f| f.hex_voice_installed = false,
                Gap::HexVoice,
                Fix::DownloadVoice,
            ),
        ];
        for (change, gap, fix) in cases {
            let mut facts = whole();
            change(&mut facts);
            assert_eq!(setup_gaps(&facts), vec![gap.clone()], "{gap:?}");
            assert_eq!(gap.fix(), fix, "{gap:?}");
        }
    }

    #[test]
    fn what_isnt_used_isnt_missing() {
        let mut facts = whole();
        facts.desktop_wanted = false;
        facts.agent_desktop_installed = false;
        facts.browser_wanted = false;
        facts.agent_browser_installed = false;
        facts.hex_voice_selected = false;
        facts.hex_voice_installed = false;
        assert!(setup_gaps(&facts).is_empty());
        // A missing agent isn't also "signed out".
        let mut gone = whole();
        gone.agent_installed = false;
        gone.agent_ready = false;
        assert_eq!(setup_gaps(&gone), vec![Gap::Agent]);
    }

    #[test]
    fn a_release_that_adds_a_step_shows_just_that_step() {
        let added = [(2, "browser"), (3, "about"), (3, "browser")];
        assert!(added_since_in(&added, 3).is_empty());
        assert_eq!(added_since_in(&added, 2), ["about", "browser"]);
        assert_eq!(added_since_in(&added, 1), ["browser", "about"]);
        // Today nothing was added after the first version.
        assert!(added_since(SETUP_VERSION).is_empty());
        let gaps = vec![Gap::NewSteps(vec!["about"])];
        assert_eq!(pages_for(&gaps, ORDER), ["about"]);
    }

    #[test]
    fn the_guide_shows_each_page_once_in_the_flows_order() {
        let gaps = vec![
            Gap::Shortcut,
            Gap::Microphone,
            Gap::ScreenRecording,
            Gap::AgentSignIn,
            Gap::HexVoice,
        ];
        assert_eq!(pages_for(&gaps, ORDER), ["agent", "desktop", "shortcut"]);
        assert!(pages_for(&[Gap::HexVoice, Gap::TranscriptionModel], ORDER).is_empty());
    }

    #[test]
    fn only_what_stops_a_question_is_said_on_a_hold() {
        assert_eq!(essential(&[Gap::Shortcut, Gap::HexVoice]), None);
        assert_eq!(
            essential(&[Gap::ScreenRecording, Gap::AgentSignIn]),
            Some(&Gap::AgentSignIn)
        );
    }

    #[test]
    fn automation_off_is_a_gap_on_the_permissions_page() {
        let facts = Facts {
            automation: false,
            ..whole()
        };
        assert_eq!(setup_gaps(&facts), [Gap::Automation]);
        assert_eq!(Gap::Automation.fix(), Fix::Page("desktop"));
        assert!(!Gap::Automation.essential());
        // Without operating apps, it isn't needed.
        let facts = Facts {
            automation: false,
            desktop_wanted: false,
            ..whole()
        };
        assert!(setup_gaps(&facts).is_empty());
    }

    #[test]
    fn data_never_asked_for_brings_back_the_permissions_page() {
        // After an update that added the asks, someone set up before sees
        // the permissions page once; answered either way, never again.
        let facts = Facts {
            personal_data: false,
            ..whole()
        };
        assert_eq!(setup_gaps(&facts), [Gap::PersonalData]);
        assert_eq!(Gap::PersonalData.fix(), Fix::Page("desktop"));
        assert!(!Gap::PersonalData.essential());
        assert_eq!(pages_for(&setup_gaps(&facts), ORDER), ["desktop"]);
    }
}
