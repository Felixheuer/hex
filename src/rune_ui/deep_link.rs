//! Links that open the window on a page: `hex://settings/<pane>` and
//! `hex://chats?id=<conversation>` (`hex-test://` in a test build). Labs is
//! gone; an old `hex://labs` link opens the Dictation pane, where most of
//! its features now live. Story links live in `story_link`.

use rune::Tx;

use super::story_link::{MAX_LINK, percent_decode};
use super::{Pane, Ui};

/// Where a link opens the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A main pane, by its link name.
    Settings(String),
    /// Chats, on a conversation (`?id=`) or a new chat.
    Chats(Option<String>),
}

/// The main panes a `hex://settings/<pane>` link names.
const PANES: [(&str, Pane); 8] = [
    ("dictation", Pane::Dictation),
    ("general", Pane::General),
    ("output", Pane::Output),
    ("history", Pane::History),
    ("chats", Pane::Chats),
    ("assistant", Pane::Assistant),
    ("meetings", Pane::Meetings),
    ("voice-filter", Pane::VoiceFilter),
];

/// Reads `scheme://settings/<pane>`, a retired `scheme://labs[…]`, and
/// `scheme://chats`. `None`
/// means it is not one of these (a story link, or not ours), for the caller
/// to read another way.
pub(crate) fn parse(link: &str, scheme: &str) -> Option<Result<Target, String>> {
    if link.len() > MAX_LINK {
        return None;
    }
    let url = url::Url::parse(link).ok()?;
    if url.scheme() != scheme {
        return None;
    }
    let host = url.host_str()?;
    if host == "chats" {
        let id = url
            .query_pairs()
            .find(|(key, value)| key == "id" && !value.trim().is_empty())
            .map(|(_, value)| value.trim().to_owned());
        return Some(Ok(Target::Chats(id)));
    }
    if host != "labs" && host != "settings" {
        return None;
    }
    let segments: Option<Vec<String>> = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .map(percent_decode)
        .collect();
    let Some(segments) = segments else {
        return Some(Err("the link isn't valid UTF-8".into()));
    };
    let name = segments.join("/").to_ascii_lowercase();
    Some(match (host, name.is_empty()) {
        ("labs", _) => Ok(Target::Settings("dictation".into())),
        (_, true) => Err("the link names no settings pane".into()),
        (_, false) => Ok(Target::Settings(name)),
    })
}

/// Opens the window's page for `target`; the caller brings the window forward.
pub(crate) fn open(ui: &mut Ui, target: Target, tx: &mut Tx) {
    match target {
        Target::Chats(conversation) => {
            super::show(ui, super::Destination::Chats(conversation), tx);
        }
        Target::Settings(name) => {
            let pane = PANES
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, p)| *p);
            match pane {
                Some(pane) if pane.shown(ui) => {
                    super::show(ui, super::Destination::Settings, tx);
                    ui.select_pane(pane, tx);
                }
                _ => {
                    tracing::warn!(pane = %name, "a settings link named no pane this Hex shows");
                    super::show(ui, super::Destination::Settings, tx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(link: &str) -> Option<Result<Target, String>> {
        parse(link, "hex")
    }

    #[test]
    fn a_retired_labs_link_opens_dictation() {
        let dictation = Some(Ok(Target::Settings("dictation".into())));
        assert_eq!(read("hex://labs"), dictation);
        assert_eq!(read("hex://labs/live-text?tab=on"), dictation);
        assert_eq!(
            parse("hex-test://labs/meetings", "hex-test"),
            Some(Ok(Target::Settings("dictation".into())))
        );
    }

    #[test]
    fn reads_settings_links() {
        assert_eq!(
            read("hex://settings/history"),
            Some(Ok(Target::Settings("history".into())))
        );
        assert!(matches!(read("hex://settings"), Some(Err(_))));
        assert!(matches!(read("hex://labs/%FF"), Some(Err(_))));
    }

    #[test]
    fn reads_chats_links() {
        assert_eq!(
            read("hex://chats?id=2f9c-41"),
            Some(Ok(Target::Chats(Some("2f9c-41".into()))))
        );
        assert_eq!(read("hex://chats"), Some(Ok(Target::Chats(None))));
        assert_eq!(read("hex://chats?id="), Some(Ok(Target::Chats(None))));
        assert_eq!(
            parse("hex-test://chats?id=a%20b", "hex-test"),
            Some(Ok(Target::Chats(Some("a b".into()))))
        );
    }

    #[test]
    fn leaves_other_links_alone() {
        for other in [
            "hex://story/hud/processing",
            "hex-test://labs/live-text",
            "https://labs/live-text",
            "hex://elsewhere",
            "not a link",
            "",
        ] {
            assert_eq!(read(other), None, "{other}");
        }
        let long = format!("hex://labs/{}", "a".repeat(MAX_LINK));
        assert_eq!(read(&long), None);
    }

    #[test]
    fn every_pane_name_is_distinct() {
        let mut names: Vec<_> = PANES.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PANES.len());
    }
}
