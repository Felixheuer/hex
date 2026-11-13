//! `hex_project`, the agent's tool for projects (`tools`): show the
//! task's project (its soul and folder), list its files, add what it
//! learned, and, in the person's own turn, update a section of the soul,
//! switch the active project, make one, or list them.
//!
//! A background job may have read something untrusted, so it only reads
//! and adds to "Recently learned" (which the person prunes); changing the
//! soul's own sections and switching are the person's turn's.

use serde_json::{Value, json};

use super::{Part, Project, Projects};

pub const NAME: &str = "hex_project";

/// The tool's definition.
pub fn definition() -> Value {
    json!({
        "name": NAME,
        "description": "The person's projects (workspaces such as „Startup“ or „privat“), each a folder with a soul.md (goal, people, sites, repos, accounts, tone, key files, decisions). This task's project, if it has one, is named in the request. Actions: \"show\" (the soul and folder), \"files\" (its newest files), \"learn\" (add one lasting fact you learned to its soul's Recently learned: a person and role, a site, a repo, an account name, the goal, a decision; never secrets), \"update\" (add a line to one section; the person's own turn only), \"list\" (every project and which is active), \"switch\" (make `project` the active one, or \"none\"; only when the person asks), \"create\" (make a project named `project`; only when the person asks).",
        "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["show", "files", "learn", "update", "list", "switch", "create"]},
            "project": {"type": "string", "description": "A project's name; default: this task's project."},
            "section": {"type": "string", "enum": ["goal", "people", "sites", "repos", "accounts", "tone", "files", "decisions"], "description": "For learn and update: what kind of fact it is."},
            "text": {"type": "string", "description": "For learn and update: the fact, one short line."}
        }, "required": ["action"]},
    })
}

/// Runs the tool for the task `project_id` belongs to (`None`: a task in
/// no project), `job` when a background job (or its helper) asks.
pub fn call(
    projects: &Projects,
    project_id: Option<&str>,
    job: bool,
    arguments: &Value,
) -> Result<String, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let action = text("action").ok_or("hex_project needs “action”")?;
    let named = text("project");
    // The project the call is about: the one named, else the task's.
    let about = || -> Result<Project, String> {
        match &named {
            Some(name) => projects.find(name).ok_or_else(|| {
                format!(
                    "There is no project “{name}”. Projects: {}.",
                    names(projects)
                )
            }),
            None => project_id
                .and_then(|id| projects.get(id))
                .ok_or_else(|| "This task isn't in a project; name one with `project`.".to_owned()),
        }
    };
    let personal = |what: &str| -> Result<(), String> {
        if job {
            Err(format!(
                "A background job can't {what} (it may have read something untrusted). Use action learn, or say it in your final answer; the person decides."
            ))
        } else {
            Ok(())
        }
    };
    match action.as_str() {
        "show" => {
            let project = about()?;
            let soul = projects.soul(&project);
            Ok(format!(
                "Project “{}”, folder {} (save what you make for it in {}).\n{}",
                project.name,
                project.folder.display(),
                project.files_folder().display(),
                cut(soul.trim(), 8000)
            ))
        }
        "files" => {
            let project = about()?;
            let files = projects.files(&project, 40);
            if files.is_empty() {
                return Ok(format!(
                    "“{}” has no files yet ({}).",
                    project.name,
                    project.folder.display()
                ));
            }
            let lines: Vec<String> = files
                .iter()
                .map(|(path, _)| format!("- {}", path.display()))
                .collect();
            Ok(format!(
                "“{}”'s newest files:\n{}",
                project.name,
                lines.join("\n")
            ))
        }
        "learn" => {
            let project = about()?;
            let fact = text("text").ok_or("learn needs “text”")?;
            let part = text("section")
                .and_then(|section| Part::parse(&section))
                .unwrap_or(Part::Learned);
            match projects.learn(&project, part, &fact) {
                Ok(true) => Ok(format!(
                    "Added to “{}”'s soul under Recently learned.",
                    project.name
                )),
                Ok(false) => Ok(format!("“{}”'s soul already has that.", project.name)),
                Err(error) => Err(format!("{error:#}")),
            }
        }
        "update" => {
            personal("change a project's soul")?;
            let project = about()?;
            let fact = text("text").ok_or("update needs “text”")?;
            let part = text("section")
                .and_then(|section| Part::parse(&section))
                .ok_or("update needs a “section”")?;
            match projects.add(&project, part, &fact) {
                Ok(true) => Ok(format!("Added to “{}”'s {}.", project.name, part.english())),
                Ok(false) => Ok(format!("“{}”'s soul already has that.", project.name)),
                Err(error) => Err(format!("{error:#}")),
            }
        }
        "list" => {
            let active = projects.active().map(|project| project.id);
            let list = projects.list();
            if list.is_empty() {
                return Ok("The person has no projects yet.".into());
            }
            Ok(list
                .iter()
                .map(|project| {
                    format!(
                        "- {}{} ({})",
                        project.name,
                        if active.as_deref() == Some(project.id.as_str()) {
                            " [active]"
                        } else {
                            ""
                        },
                        project.folder.display()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "switch" => {
            personal("switch projects")?;
            let none = named.as_deref().is_none_or(|name| {
                matches!(
                    crate::assistant::knowledge::rank::fold(name).as_str(),
                    "none" | "no project" | "kein projekt" | "ohne projekt" | "keins"
                )
            });
            if none {
                projects
                    .set_active(None)
                    .map_err(|error| format!("{error:#}"))?;
                return Ok("No project is active now; later requests are in none.".into());
            }
            let project = about()?;
            projects
                .set_active(Some(&project.id))
                .map_err(|error| format!("{error:#}"))?;
            Ok(format!(
                "“{}” is the active project for the person's next requests (this one keeps the project it started in).",
                project.name
            ))
        }
        "create" => {
            personal("make projects")?;
            let name = named.ok_or("create needs “project”, its name")?;
            let (project, made) = projects
                .create(&name)
                .map_err(|error| format!("{error:#}"))?;
            Ok(format!(
                "{} “{}” at {}. It isn't active until the person switches to it (action switch).",
                if made { "Made" } else { "There already is" },
                project.name,
                project.folder.display()
            ))
        }
        other => Err(format!("hex_project has no action “{other}”.")),
    }
}

fn names(projects: &Projects) -> String {
    let list: Vec<String> = projects
        .list()
        .into_iter()
        .map(|project| project.name)
        .collect();
    if list.is_empty() {
        "none yet".into()
    } else {
        list.join(", ")
    }
}

/// `text` cut to `max` characters, whole lines kept.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    for line in text.lines() {
        if out.chars().count() + line.chars().count() + 1 > max {
            out.push_str("…\n");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
