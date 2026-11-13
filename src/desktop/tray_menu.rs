//! What the tray menu offers: the downloaded models the macOS menu bar and
//! the Linux and Windows trays list, the assistant's status line and
//! project choice both share, and the tray menu the Linux and Windows trays
//! show. Top to bottom it reads as the menu bar does: Agent-Desk's status
//! ("Agent-Desk · 2 jobs running · 1 needs you"), the project (recent ones,
//! No project, New project…), Stop all tasks, Open Agent-Desk; the meeting
//! items; one "Dictate" section (Paste last dictation, Enable / Disable,
//! the speech model); then Settings…, the update, and Quit. Each tray maps
//! a picked [`TrayCommand`] onto its own runtime.

use crate::app_settings::AppSettings;
use crate::transcription::models::{
    ModelDefinition, TranscriptionModelId, all_models, language_name,
};

/// One downloaded model in a menu's model picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelMenuEntry {
    pub(crate) model: TranscriptionModelId,
    pub(crate) title: String,
    pub(crate) selected: bool,
    pub(crate) enabled: bool,
}

/// The available models `installed` reports downloaded, each with its
/// remembered language. None can be picked while one is `preparing`, and
/// the selected one never can.
pub(crate) fn model_menu_entries(
    settings: &AppSettings,
    preparing: bool,
    installed: impl Fn(&ModelDefinition, &str) -> bool,
) -> Vec<ModelMenuEntry> {
    all_models()
        .into_iter()
        .filter(|model| model.available())
        .filter_map(|model| {
            let selection = settings.transcription_for_model(model.id);
            installed(model, &selection.language).then(|| ModelMenuEntry {
                model: model.id,
                title: format!("{} · {}", model.name, language_name(&selection.language)),
                selected: model.id == settings.transcription.model,
                enabled: !preparing && model.id != settings.transcription.model,
            })
        })
        .collect()
}

/// What the assistant is doing, for the status line: Background jobs
/// running, tasks waiting for the person, and whether anything works.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AssistantCounts {
    pub(crate) jobs: usize,
    pub(crate) needs_you: usize,
    pub(crate) working: bool,
}

impl AssistantCounts {
    /// Counted from the service's tasks.
    pub(crate) fn of(tasks: &[crate::assistant::TaskStatus]) -> Self {
        use crate::assistant::{Lane, TaskState};
        Self {
            jobs: tasks
                .iter()
                .filter(|task| task.origin.lane() == Lane::Background)
                .count(),
            needs_you: tasks
                .iter()
                .filter(|task| task.state == TaskState::NeedsInput)
                .count(),
            working: !tasks.is_empty(),
        }
    }

    /// Read now.
    pub(crate) fn now() -> Self {
        Self::of(&crate::assistant::tasks())
    }
}

/// The menu's first line: "Agent-Desk · Ready", "Agent-Desk · 2 jobs
/// running · 1 needs you", or "Agent-Desk · Working…" while only a quick
/// turn runs.
pub(crate) fn status_line(counts: AssistantCounts) -> String {
    use crate::desktop::i18n::{tr, tr_fill};
    let mut parts = vec![crate::app_paths::app_name().to_owned()];
    match counts.jobs {
        0 => {}
        1 => parts.push(tr("1 job running").to_owned()),
        jobs => parts.push(tr_fill("{} jobs running", &jobs.to_string())),
    }
    match counts.needs_you {
        0 => {}
        1 => parts.push(tr("1 needs you").to_owned()),
        waiting => parts.push(tr_fill("{} need you", &waiting.to_string())),
    }
    if parts.len() == 1 {
        let now = if counts.working {
            tr("Working…")
        } else {
            tr("Ready")
        };
        parts.push(now.to_owned());
    }
    parts.join(" · ")
}

/// The project item's title: "Project: Startup", or "Project: No project".
pub(crate) fn project_title(snapshot: &crate::assistant::projects::Snapshot) -> String {
    use crate::desktop::i18n::{tr, tr_fill};
    let name = snapshot
        .active
        .as_ref()
        .map_or_else(|| tr("No project"), |(_, name)| name.as_str());
    tr_fill("Project: {}", name)
}

/// How many recent projects a menu lists.
pub(crate) const MENU_PROJECTS: usize = 6;

/// The projects a menu offers, most recent first: id, name, and whether it
/// is the active one; at most [`MENU_PROJECTS`].
pub(crate) fn project_entries(
    snapshot: &crate::assistant::projects::Snapshot,
) -> Vec<(String, String, bool)> {
    let active = snapshot.active.as_ref().map(|(id, _)| id.as_str());
    snapshot
        .recent
        .iter()
        .take(MENU_PROJECTS)
        .map(|(id, name)| (id.clone(), name.clone(), Some(id.as_str()) == active))
        .collect()
}

#[cfg_attr(target_os = "macos", allow(unused_imports))]
pub(crate) use tray::*;

/// The Linux and Windows trays' menu; built on macOS too, for its tests.
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod tray {
    use super::{AssistantCounts, ModelMenuEntry};
    use crate::assistant::projects::Snapshot;
    use crate::desktop::events::VoiceState;
    use crate::desktop::host::DesktopUpdateStatus;
    use crate::desktop::i18n::tr;
    use crate::transcription::models::TranscriptionModelId;

    /// What a tray menu item asks for.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum TrayCommand {
        Show,
        PasteLast,
        UseModel(TranscriptionModelId),
        /// The update item: restart into it, download it, or switch channel.
        Update,
        ToggleListening,
        Quit,
        RecordMeeting,
        /// Stop & transcribe the recording meeting.
        StopMeeting,
        CopyLastMeeting,
        /// Open Agent-Desk: the window on the Assistant pane.
        OpenChats,
        /// Stop all tasks: every assistant task and its speech stop.
        StopAllTasks,
        /// Settings…: the window on Settings.
        Settings,
        /// The recent project at this place in the menu's list becomes the
        /// active one.
        UseProject(usize),
        /// No project: none is active.
        NoProject,
        /// New project…: the overlay's picker opens with its name field.
        NewProject,
    }

    impl TrayCommand {
        /// The command behind a menu item's id.
        pub(crate) fn from_id(id: &str) -> Option<Self> {
            Some(match id {
                "show" => Self::Show,
                "paste-last" => Self::PasteLast,
                "update" => Self::Update,
                "toggle" => Self::ToggleListening,
                "quit" => Self::Quit,
                "meeting-record" => Self::RecordMeeting,
                "meeting-stop" => Self::StopMeeting,
                "meeting-copy-last" => Self::CopyLastMeeting,
                "chats" => Self::OpenChats,
                "assistant-stop-all" => Self::StopAllTasks,
                "settings" => Self::Settings,
                "project-none" => Self::NoProject,
                "project-new" => Self::NewProject,
                _ => match id.strip_prefix("project:") {
                    Some(index) => Self::UseProject(index.parse().ok()?),
                    None => Self::UseModel(id.strip_prefix("model:")?.parse().ok()?),
                },
            })
        }

        /// What a project item does, as `snapshot` lists the projects now:
        /// the id to make active, or `Some(None)` for no project. `None`
        /// for any other item, or a place no longer listed.
        pub(crate) fn project(self, snapshot: &Snapshot) -> Option<Option<String>> {
            match self {
                Self::UseProject(index) => super::project_entries(snapshot)
                    .into_iter()
                    .nth(index)
                    .map(|(id, _, _)| Some(id)),
                Self::NoProject => Some(None),
                _ => None,
            }
        }
    }

    /// Switches projects for a picked project item: makes the one it names
    /// active (or none), or opens the overlay's picker on its name field
    /// for New project…. Any other command does nothing.
    pub(crate) fn perform_project(command: TrayCommand) {
        if command == TrayCommand::NewProject {
            crate::rune_ui::assistant_overlay::open_projects(true);
            return;
        }
        let Some(id) = command.project(&crate::assistant::projects::snapshot()) else {
            return;
        };
        if let Err(error) = crate::assistant::projects::switch(id.as_deref()) {
            tracing::warn!(%error, "couldn't switch projects from the tray");
        }
    }

    /// The meeting recorder, as the tray shows it.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum MeetingTray {
        Idle,
        Starting,
        Recording,
        Transcribing,
    }

    /// The assistant's items, as the tray shows them while it's on.
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(crate) struct AssistantTray {
        /// A task runs or waits, so Stop all tasks can stop something.
        pub(crate) running: bool,
        /// What the status line counts.
        pub(crate) counts: AssistantCounts,
        /// The active project and the recent ones.
        pub(crate) projects: Snapshot,
    }

    impl AssistantTray {
        /// As the service and the projects stand now.
        pub(crate) fn now() -> Self {
            Self {
                running: crate::assistant::busy(),
                counts: AssistantCounts::now(),
                projects: crate::assistant::projects::snapshot(),
            }
        }
    }

    /// Everything the tray shows.
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub(crate) struct TrayView {
        pub(crate) listening: bool,
        pub(crate) state: Option<VoiceState>,
        pub(crate) update: Option<String>,
        pub(crate) models: Vec<ModelMenuEntry>,
        /// `None` while Meetings is off.
        pub(crate) meetings: Option<MeetingTray>,
        /// `None` while the assistant is off (`assistant::enabled`).
        pub(crate) assistant: Option<AssistantTray>,
    }

    /// A line that says something rather than does it: disabled, under an
    /// id of its own, since some tray hosts (GNOME's AppIndicators) keep an
    /// item's disabled state when the same id comes back enabled (#1287).
    fn heading(id: &str, label: impl Into<String>) -> rune::TrayItem {
        rune::TrayItem::action(id, label).enabled(false)
    }

    /// One choice of a list, ticked, or indented to line up with the tick.
    fn choice(id: String, label: &str, ticked: bool) -> rune::TrayItem {
        let mark = if ticked { "✓ " } else { "    " };
        rune::TrayItem::action(id, format!("{mark}{label}"))
    }

    impl TrayView {
        pub(crate) fn new(
            listening: bool,
            state: Option<VoiceState>,
            update: &DesktopUpdateStatus,
            models: Vec<ModelMenuEntry>,
        ) -> Self {
            Self {
                listening,
                state: state.filter(|_| listening),
                update: update_label(update),
                models,
                meetings: None,
                assistant: None,
            }
        }

        fn recording(&self) -> bool {
            self.state == Some(VoiceState::Dictating)
        }

        fn transcribing(&self) -> bool {
            self.state == Some(VoiceState::Transcribing)
        }

        pub(crate) fn tooltip(&self) -> &'static str {
            if self.recording() {
                tr("Agent-Desk Dictation: recording")
            } else if self.transcribing() {
                tr("Agent-Desk Dictation: transcribing")
            } else if self.update.is_some() {
                tr("Agent-Desk Dictation: update available")
            } else {
                tr("Agent-Desk Dictation")
            }
        }

        /// The assistant's part, while it's on: the status line, the
        /// project (the recent ones ticked where active, No project, New
        /// project…), Stop all tasks, and Open Agent-Desk.
        fn assistant_items(assistant: &AssistantTray) -> Vec<rune::TrayItem> {
            use rune::TrayItem;
            let mut menu = vec![
                heading("assistant-status", super::status_line(assistant.counts)),
                TrayItem::Separator,
                heading("project-header", super::project_title(&assistant.projects)),
            ];
            menu.extend(
                super::project_entries(&assistant.projects)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (_, name, active))| {
                        choice(format!("project:{index}"), &name, active)
                    }),
            );
            menu.push(choice(
                "project-none".into(),
                tr("No project"),
                assistant.projects.active.is_none(),
            ));
            menu.push(choice("project-new".into(), tr("New project…"), false));
            menu.push(TrayItem::Separator);
            menu.push(if assistant.running {
                TrayItem::action("assistant-stop-all", tr("Stop all tasks"))
            } else {
                heading("assistant-idle", tr("Stop all tasks"))
            });
            menu.push(TrayItem::action("chats", tr("Open Agent-Desk")));
            menu
        }

        pub(crate) fn menu(&self) -> Vec<rune::TrayItem> {
            use rune::TrayItem;
            let mut menu = match &self.assistant {
                Some(assistant) => Self::assistant_items(assistant),
                None => vec![TrayItem::action("show", tr("Show Agent-Desk"))],
            };
            if let Some(meetings) = self.meetings {
                menu.push(TrayItem::Separator);
                menu.push(match meetings {
                    MeetingTray::Idle => TrayItem::action("meeting-record", tr("Record meeting")),
                    MeetingTray::Starting => heading("meeting-starting", tr("Starting…")),
                    MeetingTray::Recording => {
                        TrayItem::action("meeting-stop", tr("Stop & transcribe"))
                    }
                    MeetingTray::Transcribing => {
                        heading("meeting-transcribing", tr("Transcribing…"))
                    }
                });
                menu.push(TrayItem::action(
                    "meeting-copy-last",
                    tr("Copy last meeting transcript"),
                ));
            }
            // Dictation, under one heading that says what it does now.
            menu.push(TrayItem::Separator);
            let dictation = if self.recording() {
                tr("Recording")
            } else if self.transcribing() {
                tr("Transcribing…")
            } else {
                tr("Dictate")
            };
            menu.push(heading("status", dictation));
            menu.push(
                TrayItem::action("paste-last", tr("Paste last dictation")).enabled(self.listening),
            );
            menu.push(TrayItem::action(
                "toggle",
                tr("Enable / Disable Agent-Desk"),
            ));
            if !self.models.is_empty() {
                menu.push(heading("models", tr("Speech model")));
                menu.extend(self.models.iter().map(|entry| {
                    choice(
                        format!("model:{}", entry.model.as_str()),
                        &entry.title,
                        entry.selected,
                    )
                    .enabled(entry.enabled)
                }));
            }
            menu.push(TrayItem::Separator);
            menu.push(TrayItem::action("settings", tr("Settings…")));
            menu.extend(
                self.update
                    .as_ref()
                    .map(|label| TrayItem::action("update", label.clone())),
            );
            menu.push(TrayItem::action("quit", tr("Quit Agent-Desk")));
            menu
        }
    }

    /// The update the window's sidebar offers: Restart to update once a
    /// managed install has staged it, Download update for an install that
    /// cannot apply it itself, or the channel switch.
    fn update_label(status: &DesktopUpdateStatus) -> Option<String> {
        match status {
            DesktopUpdateStatus::ReadyToRestart => Some(tr("Restart to update").to_owned()),
            DesktopUpdateStatus::Available => Some(tr("Download update").to_owned()),
            DesktopUpdateStatus::SwitchAvailable { channel, version } => {
                Some(crate::updater::switch_label(channel, version))
            }
            _ => None,
        }
    }

    /// What the update item does for `status`, if it shows.
    pub(crate) fn update_action(
        status: &DesktopUpdateStatus,
    ) -> Option<crate::desktop::host::DesktopAction> {
        use crate::desktop::host::DesktopAction;
        match status {
            DesktopUpdateStatus::ReadyToRestart => Some(DesktopAction::RestartIntoUpdate),
            DesktopUpdateStatus::Available => Some(DesktopAction::OpenUpdate),
            DesktopUpdateStatus::SwitchAvailable { .. } => Some(DesktopAction::ConfirmUpdateSwitch),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::projects::Snapshot;
    use crate::transcription::models::TranscriptionSelection;

    #[test]
    fn model_menu_only_lists_downloaded_models_with_their_remembered_language() {
        let mut settings = AppSettings::default();
        settings.remember_transcription(TranscriptionSelection {
            model: TranscriptionModelId::WhisperLargeV3Turbo,
            language: "fr".into(),
        });
        settings.remember_transcription(TranscriptionSelection::default());
        let installed = |model: &ModelDefinition, _: &str| {
            matches!(
                model.id,
                TranscriptionModelId::ParakeetUnifiedEnglish
                    | TranscriptionModelId::WhisperLargeV3Turbo
            )
        };
        let entries = model_menu_entries(&settings, false, installed);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Parakeet Unified English · English");
        assert!(entries[0].selected);
        assert!(!entries[0].enabled);
        assert_eq!(entries[1].title, "Whisper large-v3-turbo · French");
        assert!(!entries[1].selected);
        assert!(entries[1].enabled);
        let preparing = model_menu_entries(&settings, true, installed);
        assert!(preparing.iter().all(|entry| !entry.enabled));
        assert!(
            preparing[0].selected,
            "preparing must not mark the candidate selected"
        );
        assert!(model_menu_entries(&settings, false, |_, _| false).is_empty());
    }

    fn projects(active: Option<&str>) -> Snapshot {
        Snapshot {
            active: active.map(|name| (format!("id-{name}"), name.to_owned())),
            recent: ["Startup", "Privat"]
                .into_iter()
                .map(|name| (format!("id-{name}"), name.to_owned()))
                .collect(),
        }
    }

    #[test]
    fn the_status_line_counts_jobs_and_who_needs_you() {
        let app = crate::app_paths::app_name();
        let line = |jobs, needs_you, working| {
            status_line(AssistantCounts {
                jobs,
                needs_you,
                working,
            })
        };
        assert_eq!(line(0, 0, false), format!("{app} · Ready"));
        assert_eq!(line(0, 0, true), format!("{app} · Working…"));
        assert_eq!(line(1, 0, true), format!("{app} · 1 job running"));
        assert_eq!(
            line(2, 1, true),
            format!("{app} · 2 jobs running · 1 needs you")
        );
        assert_eq!(line(0, 3, true), format!("{app} · 3 need you"));
    }

    #[test]
    fn the_counts_read_jobs_by_their_lane_and_questions_by_their_state() {
        use crate::assistant::{Origin, TaskState, TaskStatus};
        let task = |origin, state| TaskStatus {
            conversation: "c".into(),
            origin,
            title: String::new(),
            state,
            step: None,
            windows: Vec::new(),
            colour: 0,
        };
        let counts = AssistantCounts::of(&[
            task(Origin::Background, TaskState::Running),
            task(Origin::Chat, TaskState::NeedsInput),
            task(Origin::ActionShortcut, TaskState::Running),
        ]);
        assert_eq!(
            counts,
            AssistantCounts {
                jobs: 2,
                needs_you: 1,
                working: true
            }
        );
        assert_eq!(AssistantCounts::of(&[]), AssistantCounts::default());
    }

    #[test]
    fn the_project_item_names_the_active_one_and_lists_the_recent() {
        assert_eq!(
            project_title(&projects(Some("Startup"))),
            "Project: Startup"
        );
        assert_eq!(project_title(&projects(None)), "Project: No project");
        let entries = project_entries(&projects(Some("Privat")));
        assert_eq!(
            entries,
            [
                ("id-Startup".into(), "Startup".into(), false),
                ("id-Privat".into(), "Privat".into(), true),
            ]
        );
        let many = Snapshot {
            active: None,
            recent: (0..10).map(|n| (n.to_string(), n.to_string())).collect(),
        };
        assert_eq!(project_entries(&many).len(), MENU_PROJECTS);
    }

    fn ids(view: &TrayView) -> Vec<String> {
        view.menu()
            .into_iter()
            .filter_map(|item| match item {
                rune::TrayItem::Action { id, .. } => Some(id),
                _ => None,
            })
            .collect()
    }

    fn label(view: &TrayView, wanted: &str) -> Option<(String, bool)> {
        view.menu().into_iter().find_map(|item| match item {
            rune::TrayItem::Action { id, label, enabled } if id == wanted => Some((label, enabled)),
            _ => None,
        })
    }

    #[test]
    fn tray_menu_ids_round_trip_and_show_the_state() {
        use crate::desktop::events::VoiceState;
        use crate::desktop::host::DesktopUpdateStatus;
        let settings = AppSettings::default();
        let models = model_menu_entries(&settings, false, |model, _| {
            model.id == settings.transcription.model
        });
        let view = TrayView::new(
            true,
            Some(VoiceState::Dictating),
            &DesktopUpdateStatus::ReadyToRestart,
            models,
        );
        let model_id = format!("model:{}", settings.transcription.model.as_str());
        // The assistant off: Show, then the dictation section, then the app.
        assert_eq!(
            ids(&view),
            [
                "show",
                "status",
                "paste-last",
                "toggle",
                "models",
                model_id.as_str(),
                "settings",
                "update",
                "quit"
            ]
        );
        // The dictation section's heading says what it does now.
        assert_eq!(label(&view, "status"), Some(("Recording".into(), false)));
        let idle = TrayView::new(true, None, &DesktopUpdateStatus::Current, Vec::new());
        assert_eq!(label(&idle, "status"), Some(("Dictate".into(), false)));
        assert_eq!(
            TrayCommand::from_id(&model_id),
            Some(TrayCommand::UseModel(settings.transcription.model))
        );
        assert_eq!(
            TrayCommand::from_id("paste-last"),
            Some(TrayCommand::PasteLast)
        );
        assert_eq!(
            TrayCommand::from_id("settings"),
            Some(TrayCommand::Settings)
        );
        assert_eq!(TrayCommand::from_id("status"), None);
        assert_eq!(view.tooltip(), "Agent-Desk Dictation: recording");
        // Meetings on adds its items before the dictation section.
        let meeting = TrayView {
            meetings: Some(MeetingTray::Recording),
            ..view.clone()
        };
        assert_eq!(
            ids(&meeting)[1..4],
            ["meeting-stop", "meeting-copy-last", "status"]
        );
        assert_eq!(
            TrayCommand::from_id("meeting-stop"),
            Some(TrayCommand::StopMeeting)
        );
        // Disabled meeting states never reuse an enabled item's id (#1287).
        let meeting_id = |state| {
            ids(&TrayView {
                meetings: Some(state),
                ..view.clone()
            })
            .into_iter()
            .find(|id| id.starts_with("meeting-"))
            .unwrap()
        };
        let enabled = [
            meeting_id(MeetingTray::Idle),
            meeting_id(MeetingTray::Recording),
        ];
        for disabled in [MeetingTray::Starting, MeetingTray::Transcribing] {
            assert!(!enabled.contains(&meeting_id(disabled)));
        }
        let stopped = TrayView::new(
            false,
            Some(VoiceState::Dictating),
            &DesktopUpdateStatus::Current,
            Vec::new(),
        );
        assert_eq!(stopped.tooltip(), "Agent-Desk Dictation");
    }

    #[test]
    fn the_assistant_heads_the_tray_with_its_status_and_project() {
        use crate::desktop::host::DesktopUpdateStatus;
        let base = TrayView::new(true, None, &DesktopUpdateStatus::Current, Vec::new());
        let running = TrayView {
            assistant: Some(AssistantTray {
                running: true,
                counts: AssistantCounts {
                    jobs: 2,
                    needs_you: 1,
                    working: true,
                },
                projects: projects(Some("Startup")),
            }),
            ..base.clone()
        };
        assert_eq!(
            ids(&running)[..8],
            [
                "assistant-status",
                "project-header",
                "project:0",
                "project:1",
                "project-none",
                "project-new",
                "assistant-stop-all",
                "chats"
            ]
        );
        assert!(
            !ids(&running).iter().any(|id| id == "show"),
            "Open replaces Show"
        );
        let app = crate::app_paths::app_name();
        assert_eq!(
            label(&running, "assistant-status"),
            Some((format!("{app} · 2 jobs running · 1 needs you"), false))
        );
        assert_eq!(
            label(&running, "project-header"),
            Some(("Project: Startup".into(), false))
        );
        assert_eq!(
            label(&running, "project:0"),
            Some(("✓ Startup".into(), true))
        );
        assert_eq!(
            label(&running, "project:1"),
            Some(("    Privat".into(), true))
        );
        assert_eq!(
            label(&running, "project-none"),
            Some(("    No project".into(), true))
        );
        // Nothing runs: Stop all tasks is disabled under an id of its own
        // (#1287), and No project is ticked without one.
        let idle = TrayView {
            assistant: Some(AssistantTray {
                projects: projects(None),
                ..AssistantTray::default()
            }),
            ..base
        };
        assert!(ids(&idle).iter().any(|id| id == "assistant-idle"));
        assert_eq!(
            label(&idle, "project-none"),
            Some(("✓ No project".into(), true))
        );
        assert_eq!(TrayCommand::from_id("chats"), Some(TrayCommand::OpenChats));
        assert_eq!(
            TrayCommand::from_id("assistant-stop-all"),
            Some(TrayCommand::StopAllTasks)
        );
        assert_eq!(TrayCommand::from_id("assistant-idle"), None);
        assert_eq!(TrayCommand::from_id("project-header"), None);
        // Project items resolve against the projects as listed now.
        let listed = projects(Some("Startup"));
        assert_eq!(
            TrayCommand::from_id("project:1").unwrap().project(&listed),
            Some(Some("id-Privat".into()))
        );
        assert_eq!(
            TrayCommand::from_id("project-none")
                .unwrap()
                .project(&listed),
            Some(None)
        );
        assert_eq!(TrayCommand::UseProject(9).project(&listed), None);
        assert_eq!(
            TrayCommand::from_id("project-new"),
            Some(TrayCommand::NewProject)
        );
        assert_eq!(TrayCommand::Settings.project(&listed), None);
    }
}
