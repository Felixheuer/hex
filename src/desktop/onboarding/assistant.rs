//! Where the assistant's onboarding pages stand, kept beside the voice
//! tour's own markers (`onboarding-step`, `onboarding-relaunch`) so a run
//! cut short (a restart macOS asks for after a grant, Restart Hex and
//! continue, ⌘Q) reopens on the page it was on. Only the page is kept:
//! every page checks again when it shows, so nothing counts as done until
//! its check passes after the relaunch.

use std::fs;
use std::path::Path;

const MARKER: &str = "assistant-onboarding";

/// The assistant onboarding's saved place.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantProgress {
    /// The page's stable name (`agent`, `desktop`, `browser`, …).
    pub page: String,
    /// The person closed it: it waits in Chats instead of reopening at
    /// launch.
    pub dismissed: bool,
}

/// Remembers `progress`; `None` forgets it (setup finished).
pub fn record_assistant(progress: Option<&AssistantProgress>) {
    let result = crate::app_paths::support_dir()
        .and_then(|directory| record_assistant_at(&directory, progress));
    if let Err(error) = result {
        tracing::warn!(%error, "could not record the assistant onboarding's page");
    }
}

/// The saved place, if a run was cut short.
pub fn saved_assistant() -> Option<AssistantProgress> {
    saved_assistant_at(&crate::app_paths::support_dir().ok()?)
}

pub(crate) fn record_assistant_at(
    directory: &Path,
    progress: Option<&AssistantProgress>,
) -> color_eyre::Result<()> {
    let path = directory.join(MARKER);
    match progress {
        Some(progress) => {
            fs::create_dir_all(directory)?;
            let state = if progress.dismissed { "dismissed" } else { "open" };
            fs::write(path, format!("{}\n{state}\n", progress.page))?;
        }
        None => match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.into());
            }
            _ => {}
        },
    }
    Ok(())
}

pub(crate) fn saved_assistant_at(directory: &Path) -> Option<AssistantProgress> {
    let saved = fs::read_to_string(directory.join(MARKER)).ok()?;
    let mut lines = saved.lines().map(str::trim);
    let page = lines.next().filter(|page| !page.is_empty())?.to_owned();
    Some(AssistantProgress {
        page,
        dismissed: lines.next() == Some("dismissed"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_assistant_page_survives_a_relaunch_until_setup_finishes() {
        let directory =
            std::env::temp_dir().join(format!("hex-assistant-progress-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        assert_eq!(saved_assistant_at(&directory), None);
        let progress = AssistantProgress {
            page: "desktop".into(),
            dismissed: false,
        };
        record_assistant_at(&directory, Some(&progress)).unwrap();
        assert_eq!(saved_assistant_at(&directory), Some(progress.clone()));
        let closed = AssistantProgress {
            dismissed: true,
            ..progress
        };
        record_assistant_at(&directory, Some(&closed)).unwrap();
        assert_eq!(saved_assistant_at(&directory), Some(closed));
        record_assistant_at(&directory, None).unwrap();
        assert_eq!(saved_assistant_at(&directory), None);
        record_assistant_at(&directory, None).unwrap();
        let _ = fs::remove_dir_all(&directory);
    }
}
