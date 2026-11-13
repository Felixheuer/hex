//! Assistant settings › Projects (`assistant::projects`): every project
//! with its folder, Use (or Leave, for the active one), Open folder, Edit
//! soul, and Archive; a field to make a new one; the archived ones to
//! restore. Built from the window's existing rows and buttons.

use rune::{El, TextEdit, row};
use rune_kit::{Button, ButtonVariant, TextInput};

use super::super::super::controls::{CardRow, Paint, PushSize, section, setting_row};
use super::super::Ui;
use crate::assistant::projects::{Project, Projects};
use crate::desktop::i18n::tr;

/// The section's lists and its name field.
#[derive(Default)]
pub(in crate::rune_ui) struct ProjectsState {
    /// Every project, archived ones too, as last read.
    pub(in crate::rune_ui) all: Vec<Project>,
    /// The active project's id.
    pub(in crate::rune_ui) active: Option<String>,
    /// A new project's name, as typed.
    pub(in crate::rune_ui) name: TextEdit,
    /// Why the last action didn't work.
    pub(in crate::rune_ui) error: Option<String>,
}

/// Settings opened (or a project changed): the projects are read again.
pub(super) fn opened(ui: &mut Ui) {
    let state = &mut ui.chats.settings.projects;
    match Projects::system() {
        Some(projects) => {
            state.all = projects.all();
            state.active = projects.active().map(|project| project.id);
        }
        None => {
            state.all.clear();
            state.active = None;
        }
    }
}

/// Runs `change` on the person's projects, then reads them again; a
/// failure shows under the section.
fn change(ui: &mut Ui, change: impl FnOnce(&Projects) -> color_eyre::Result<()>) {
    let result = Projects::system()
        .ok_or_else(|| color_eyre::eyre::eyre!("Agent-Desk has no folder for projects"))
        .and_then(|projects| change(&projects));
    ui.chats.settings.projects.error = result.err().map(|error| format!("{error:#}"));
    opened(ui);
}

/// Opens `path` in its app (a folder in Finder, the soul in the text
/// editor), off the window's thread.
fn open(path: std::path::PathBuf, text: bool) {
    let _ = std::thread::Builder::new()
        .name("agent-desk-open-project".into())
        .spawn(move || {
            #[cfg(target_os = "macos")]
            if text {
                let _ = crate::desktop::background_process::command("/usr/bin/open")
                    .arg("-t")
                    .arg(&path)
                    .status();
                return;
            }
            let _ = text;
            if let Err(error) = crate::desktop::opencode_catalog::open_url(&path.to_string_lossy())
            {
                tracing::warn!(%error, "couldn't open a project's folder");
            }
        });
}

/// `~/…` for a folder in the home folder.
fn shown(path: &std::path::Path) -> String {
    match crate::app_paths::home_dir()
        .and_then(|home| path.strip_prefix(home).ok().map(|rest| rest.to_path_buf()))
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn small(
    p: Paint,
    key: String,
    words: &str,
    variant: ButtonVariant,
    on_press: impl Fn(&mut Ui) + 'static,
) -> El {
    Button::new(words)
        .variant(variant)
        .on_press(on_press)
        .build_small_push(p.k)
        .key(key)
}

/// One project's row: its name (and "Active"), its folder, and its
/// buttons.
fn project_row(p: Paint, project: &Project, active: bool) -> El {
    let id = project.id.clone();
    let use_it = {
        let id = id.clone();
        if active {
            small(
                p,
                format!("project-leave-{id}"),
                tr("Leave"),
                ButtonVariant::Neutral,
                |ui: &mut Ui| {
                    change(ui, |projects| projects.set_active(None).map(|_| ()));
                },
            )
        } else {
            small(
                p,
                format!("project-use-{id}"),
                tr("Use"),
                ButtonVariant::Neutral,
                move |ui: &mut Ui| {
                    change(ui, |projects| projects.set_active(Some(&id)).map(|_| ()));
                },
            )
        }
    };
    let folder = project.folder.clone();
    let soul = project.soul_path();
    let archive = id.clone();
    let controls = row()
        .gap(6.0)
        .center_y()
        .child(use_it)
        .child(small(
            p,
            format!("project-folder-{id}"),
            tr("Open folder"),
            ButtonVariant::Ghost,
            move |_| open(folder.clone(), false),
        ))
        .child(small(
            p,
            format!("project-soul-{id}"),
            tr("Edit soul"),
            ButtonVariant::Ghost,
            move |_| open(soul.clone(), true),
        ))
        .child(small(
            p,
            format!("project-archive-{id}"),
            tr("Archive"),
            ButtonVariant::Ghost,
            move |ui: &mut Ui| {
                let id = archive.clone();
                change(ui, |projects| projects.set_archived(&id, true));
            },
        ));
    let title = if active {
        format!("● {} · {}", project.name, tr("Active"))
    } else {
        project.name.clone()
    };
    setting_row(
        p,
        &format!("project-{}", project.id),
        &title,
        Some(&shown(&project.folder)),
        controls,
    )
}

/// Assistant settings › Projects.
pub(in crate::rune_ui) fn section_for(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings.projects;
    let mut rows: Vec<CardRow> = Vec::new();
    for project in state.all.iter().filter(|project| !project.archived) {
        rows.push(
            project_row(
                p,
                project,
                state.active.as_deref() == Some(project.id.as_str()),
            )
            .into(),
        );
    }
    let typed = state.name.text.trim().to_owned();
    let create = move |ui: &mut Ui| {
        let name = ui.chats.settings.projects.name.text.trim().to_owned();
        if name.is_empty() {
            return;
        }
        change(ui, |projects| {
            let (project, _) = projects.create(&name)?;
            projects.set_active(Some(&project.id)).map(|_| ())
        });
        if ui.chats.settings.projects.error.is_none() {
            ui.chats.settings.projects.name = TextEdit::default();
        }
    };
    let field = TextInput::new(&state.name)
        .placeholder(tr("Name of the project"))
        .label(tr("Name of the project"))
        .width(Some(200.0))
        .on_edit(|ui: &mut Ui| &mut ui.chats.settings.projects.name)
        .build(p.k)
        .key("project-name-field");
    let make = Button::new(tr("Create"))
        .variant(ButtonVariant::Neutral)
        .disabled(typed.is_empty())
        .on_press(create)
        .build_small_push(p.k)
        .key("project-create");
    let description = match &state.error {
        Some(error) => error.clone(),
        None => tr("Each gets a folder in Documents › Agent-Desk › Projects and a soul Agent-Desk fills as it works. Without one, everything works as always").to_owned(),
    };
    rows.push(
        setting_row(
            p,
            "project-new",
            tr("New project"),
            Some(&description),
            row().gap(6.0).center_y().child(field).child(make),
        )
        .into(),
    );
    for project in state.all.iter().filter(|project| project.archived) {
        let id = project.id.clone();
        rows.push(
            setting_row(
                p,
                &format!("project-archived-{}", project.id),
                &project.name,
                Some(tr("Archived")),
                small(
                    p,
                    format!("project-restore-{id}"),
                    tr("Restore"),
                    ButtonVariant::Ghost,
                    move |ui: &mut Ui| {
                        let id = id.clone();
                        change(ui, |projects| projects.set_archived(&id, false));
                    },
                ),
            )
            .into(),
        );
    }
    section(p, "assistant-projects", tr("Projects"), rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::AppSettings;
    use crate::rune_ui::{Service, offscreen};
    use rune::text::TextSystem;
    use rune::{Cx, Env, Runtime};

    const SIZE: (f32, f32) = (720.0, 470.0);

    fn project(name: &str, archived: bool) -> Project {
        Project {
            id: format!("id-{name}"),
            name: name.into(),
            folder: std::path::PathBuf::from(format!(
                "/Users/person/Documents/Agent-Desk/Projects/{name}"
            )),
            archived,
            created_ms: 1,
        }
    }

    fn page(ui: &Ui, cx: &mut Cx<Ui>) -> El {
        let p = Paint::new(cx.system_dark(), cx.env().scale);
        rune::col()
            .key("page")
            .w_full()
            .h_full()
            .padding(20.0)
            .fill(p.k.c.bg_base)
            .child(section_for(ui, p))
    }

    fn runtime(dark: bool) -> Runtime<Ui> {
        let mut ui = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
        let state = &mut ui.chats.settings.projects;
        state.all = vec![
            project("Kalibermarkt", false),
            project("Privat", false),
            project("Startup", false),
            project("Steuer 2024", true),
        ];
        state.active = Some("id-Startup".into());
        state.name = TextEdit::new("Garten");
        let env = Env {
            system_dark: dark,
            scale: 2.0,
        };
        let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
        let mut rt = offscreen::runtime(ui, page, SIZE, env, text);
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn has(rt: &Runtime<Ui>, key: &str) -> bool {
        rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some(key))
    }

    #[test]
    fn each_project_has_its_buttons_and_the_archived_one_restores() {
        for dark in [false, true] {
            let rt = runtime(dark);
            for name in ["Kalibermarkt", "Privat", "Startup"] {
                for button in ["folder", "soul", "archive"] {
                    assert!(
                        has(&rt, &format!("project-{button}-id-{name}")),
                        "{button} {name}"
                    );
                }
            }
            assert!(
                has(&rt, "project-leave-id-Startup"),
                "the active one can be left"
            );
            assert!(has(&rt, "project-use-id-Privat"));
            assert!(!has(&rt, "project-use-id-Startup"));
            assert!(has(&rt, "project-restore-id-Steuer 2024"));
            assert!(
                !has(&rt, "project-use-id-Steuer 2024"),
                "archived: only Restore"
            );
            assert!(has(&rt, "project-create") && has(&rt, "project-name-field"));
        }
    }

    /// Renders the section to  in both appearances.
    #[test]
    #[ignore = "set AGENT_DESK_PROJECT_SNAPS to a folder; needs a GPU"]
    fn settings_projects_snapshots() {
        let folder = std::path::PathBuf::from(std::env::var("AGENT_DESK_PROJECT_SNAPS").unwrap());
        std::fs::create_dir_all(&folder).unwrap();
        let gpu = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None).unwrap();
        for dark in [false, true] {
            let mut rt = runtime(dark);
            let (w, h) = ((SIZE.0 * 2.0) as u32, (SIZE.1 * 2.0) as u32);
            let pixels = offscreen::draw(&gpu, &mut rt, w, h);
            let theme = if dark { "dark" } else { "light" };
            rune::render::write_png(
                &folder.join(format!("project-settings-{theme}.png")),
                w,
                h,
                &pixels,
            )
            .unwrap();
        }
    }
}
