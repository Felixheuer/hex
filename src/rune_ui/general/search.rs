//! The Dictation pane's assembly: permissions and errors, the listener and
//! desktop shortcut cards, the basics, what happens to the text, then the
//! Advanced fold's Dictation and Output sections. Also [`Rows`], which the
//! sections build their rows with (a query keeps only matching rows; the
//! panes pass none).

use rune::{El, row};

use super::super::controls::Paint;
use super::super::controls::{CardRow, icon_row, page, section};
use super::super::icons::CIRCLE_X;
use super::super::platform_settings as os;
use super::super::settings as s;
use super::super::{Pane, ShortcutTarget, Ui, scroll_body};
use super::{advanced_open, advanced_toggle, basics, dictation, output, shortcut_rows, text_rows};
use crate::desktop::i18n::tr;

/// The Dictation pane.
pub(crate) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let mut content = page("content")
        .children(os::permissions_section(ui, p))
        .children(os::error_section(ui, p));
    if ui.platform.caps.listener_control {
        let (_, card) = s::listener_card(ui, p);
        content = content.child(card);
    }
    if !ui.records_shortcuts() {
        content = content.child(s::desktop_commands(ui, p));
    }
    if let Some(basics) = basics(ui, p, layer, "") {
        content = content.child(basics);
    }
    content = content.child(section(p, "text", tr("Text"), text_rows(ui, p, layer, "")));
    content = content.child(advanced_toggle(ui, p, Pane::Dictation));
    if advanced_open(ui, Pane::Dictation) {
        for (key, title, rows) in [
            (
                "more-dictation",
                tr("Dictation"),
                dictation(ui, p, layer, ""),
            ),
            ("output", tr("Output"), output(ui, p, layer, "")),
        ] {
            if !rows.is_empty() {
                content = content.child(section(p, key, title, rows));
            }
        }
    }
    scroll_body(p.k, "scroll", content)
}

/// Toggle's cancel shortcut. A shortcuts portal would hold its key from
/// every app for as long as Hex runs, so on Wayland it says to bind
/// `hex dictation cancel` in the desktop instead. Its description comes back
/// for search too.
pub(super) fn cancel_rows(ui: &Ui, p: Paint) -> (&'static str, Vec<CardRow>) {
    if matches!(
        ui.shortcuts(),
        Some(crate::desktop::host::DesktopShortcuts::Portal(_))
    ) {
        let description =
            tr("Bind hex dictation cancel to a key in your desktop's keyboard shortcuts");
        return (
            description,
            vec![
                icon_row(
                    p,
                    "cancel-shortcut",
                    &CIRCLE_X,
                    tr("Cancel"),
                    Some(description),
                    row().key("cancel-shortcut-control"),
                )
                .into(),
            ],
        );
    }
    let description = tr("Discards the recording");
    (
        description,
        shortcut_rows(
            ui,
            p,
            "cancel-shortcut",
            &CIRCLE_X,
            tr("Cancel"),
            ShortcutTarget::Cancel,
            Some(description),
        ),
    )
}

/// Rows under construction, kept only while their title, or a description
/// given to [`Rows::push_described`], contains the query.
pub(super) struct Rows<'a> {
    query: &'a str,
    rows: Vec<CardRow>,
}

impl<'a> Rows<'a> {
    /// Every row shows while `query` is empty.
    pub(super) fn new(query: &'a str) -> Self {
        Self {
            query,
            rows: Vec::new(),
        }
    }

    /// Keeps `row` when its title matches.
    pub(super) fn push(&mut self, title: &str, row: impl Into<CardRow>) {
        self.push_described(title, None, row);
    }

    /// Keeps `row` when its title or `description` matches, and skips an
    /// empty fold while searching: it takes no space, so it is no match.
    pub(super) fn push_described(
        &mut self,
        title: &str,
        description: Option<&str>,
        row: impl Into<CardRow>,
    ) {
        let row = row.into();
        if matches(self.query, title, description) && (self.query.is_empty() || !empty_fold(&row)) {
            self.rows.push(row);
        }
    }

    /// Keeps `rows` together when `title` matches.
    pub(super) fn extend(&mut self, title: &str, rows: impl IntoIterator<Item = CardRow>) {
        self.extend_described(title, None, rows);
    }

    /// Keeps `rows` together when `title` or `description` matches, without
    /// the empty folds while searching.
    pub(super) fn extend_described(
        &mut self,
        title: &str,
        description: Option<&str>,
        rows: impl IntoIterator<Item = CardRow>,
    ) {
        if matches(self.query, title, description) {
            let rows = rows.into_iter();
            if self.query.is_empty() {
                self.rows.extend(rows);
            } else {
                self.rows.extend(rows.filter(|row| !empty_fold(row)));
            }
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn into_rows(self) -> Vec<CardRow> {
        self.rows
    }
}

/// A fold with nothing under it draws nothing.
fn empty_fold(row: &CardRow) -> bool {
    matches!(row, CardRow::Folding { rows, .. } if rows.is_empty())
}

/// Whether a case-insensitive `query` appears in `title` or `description`;
/// an empty query matches every row.
pub(super) fn matches(query: &str, title: &str, description: Option<&str>) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || title.to_lowercase().contains(&query)
        || description.is_some_and(|description| description.to_lowercase().contains(&query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_titles_and_descriptions_ignoring_case_and_spaces() {
        assert!(matches("mic", "Microphone", None));
        assert!(matches("MIC", "Microphone", None));
        assert!(matches("  microphone  ", "Microphone", None));
        assert!(matches("model", "Speech model", Some("Download a model")));
        assert!(matches(
            "download",
            "Speech model",
            Some("Download a model")
        ));
        assert!(!matches("model", "Microphone", None));
        assert!(!matches("model", "Microphone", Some("System input")));
        assert!(matches("", "Microphone", None));
    }
}
