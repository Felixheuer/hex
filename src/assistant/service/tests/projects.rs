//! Projects in the runner: none changes nothing; the active one gives a
//! task its soul and folder and learns from it; spoken commands switch
//! without an agent; a request about a project is offered to it, never
//! moved.

use super::*;
use crate::assistant::projects::{Part, Projects};

/// A world with projects: "Startup" (its site startup.io) and
/// "Kalibermarkt" (kalibermarkt.de), none active.
fn with_projects(world: &World) -> Projects {
    let projects = Projects::at(
        world.workspaces.join("Projects"),
        world.workspaces.join("projects.json"),
    );
    let (startup, _) = projects.create("Startup").unwrap();
    projects
        .add(&startup, Part::Goal, "Ship the beta in May")
        .unwrap();
    projects.add(&startup, Part::Sites, "startup.io").unwrap();
    let (shop, _) = projects.create("Kalibermarkt").unwrap();
    projects.add(&shop, Part::Sites, "kalibermarkt.de").unwrap();
    *world.projects.lock().unwrap() = Some(projects.clone());
    projects
}

#[test]
fn without_a_project_a_task_runs_exactly_as_before() {
    let world = World::new(8);
    let projects = with_projects(&world);
    let runner = world.runner();
    let id = runner.submit(request("Sortiere meine Downloads", Origin::Background));
    let job = world.job_for("Sortiere meine Downloads");
    assert_eq!(job.project, None);
    let rules = crate::assistant::agent::request_rules(&job);
    assert!(!rules.contains("project"), "{rules}");
    let workspace = job.workspace.expect("a job has a folder");
    assert!(
        !workspace.starts_with(&projects.root),
        "{}",
        workspace.display()
    );
    world.release("Sortiere meine Downloads");
    world.wait_for("the answer", ended(&id));
    assert_eq!(
        world.count(|event| matches!(
            event,
            Event::InProject { .. } | Event::ProjectSuggested { .. }
        )),
        0
    );
}

#[test]
fn in_the_active_project_a_task_gets_its_soul_its_folder_and_feeds_it() {
    let world = World::new(8);
    let projects = with_projects(&world);
    let startup = projects.find("Startup").unwrap();
    projects.set_active(Some(&startup.id)).unwrap();
    let runner = world.runner();
    let words = "Bau eine Preisseite für startup.io und schreib an anna@startup.io";
    let id = runner.submit(request(words, Origin::Background));
    let job = world.job_for(words);
    let project = job.project.clone().expect("the active project");
    assert_eq!(project.name, "Startup");
    let rules = crate::assistant::agent::request_rules(&job);
    assert!(rules.contains("project “Startup”"), "{rules}");
    assert!(rules.contains("- Ship the beta in May"), "{rules}");
    // The system prompt doesn't change: a backend started ahead stays warm.
    let mut without = job.clone();
    without.project = None;
    assert_eq!(
        crate::assistant::agent::system_for(&job, None, false),
        crate::assistant::agent::system_for(&without, None, false)
    );
    let workspace = job.workspace.clone().expect("a job has a folder");
    assert!(
        workspace.starts_with(startup.jobs_folder()),
        "{}",
        workspace.display()
    );
    std::fs::write(workspace.join("Preise.html"), "<h1>Preise</h1>").unwrap();
    world.wait_for("its project's tag", |events| {
        events.iter().any(|event| {
            matches!(event, Event::InProject { conversation, name } if *conversation == id && name == "Startup")
        })
    });
    world.release(words);
    world.wait_for("the answer", ended(&id));
    let soul = projects.soul(&startup);
    assert!(soul.contains("- People: anna@startup.io"), "{soul}");
    assert!(soul.contains("Preise.html"), "what the job made: {soul}");
    assert!(
        !soul.contains("- Websites: startup.io"),
        "the soul had it already: {soul}"
    );
    // A follow-up keeps its project after another is chosen.
    world
        .project_of
        .lock()
        .unwrap()
        .insert(id.clone(), startup.id.clone());
    projects.set_active(None).unwrap();
    runner.submit(Request {
        conversation: Some(id.clone()),
        ..request("Und eine englische Version", Origin::Chat)
    });
    let follow_up = world.job_for("Und eine englische Version");
    assert_eq!(
        follow_up.project.map(|project| project.name).as_deref(),
        Some("Startup")
    );
    runner.cancel_all();
}

#[test]
fn saying_a_project_command_switches_without_asking_an_agent() {
    let world = World::new(8);
    let projects = with_projects(&world);
    let runner = world.runner();
    let active = || projects.active().map(|project| project.name);
    runner.submit(request(
        "Wechsle zu Projekt Startup",
        Origin::ActionShortcut,
    ));
    assert_eq!(active().as_deref(), Some("Startup"));
    runner.submit(request("Neues Projekt Garten", Origin::ActionShortcut));
    assert_eq!(active().as_deref(), Some("Garten"));
    assert!(projects.root.join("Garten").join("soul.md").is_file());
    runner.submit(request("Ohne Projekt", Origin::ActionShortcut));
    assert_eq!(active(), None);
    runner.submit(request(
        "switch to project Gartenzwerge im Winter",
        Origin::ActionShortcut,
    ));
    assert_eq!(active(), None, "no such project");
    assert!(world.jobs.lock().unwrap().is_empty(), "no agent was asked");
    assert_eq!(
        world.count(|event| matches!(event, Event::Answer { .. })),
        4
    );
}

#[test]
fn a_request_about_a_project_while_none_is_active_is_offered_to_it_not_moved() {
    let world = World::new(8);
    let projects = with_projects(&world);
    let runner = world.runner();
    let words = "Wie viele Bestellungen hatte kalibermarkt.de heute?";
    let id = runner.submit(request(words, Origin::ActionShortcut));
    let job = world.job_for(words);
    assert_eq!(job.project, None, "never switched silently");
    world.wait_for("the offer", |events| {
        events.iter().any(|event| {
            matches!(event, Event::ProjectSuggested { conversation, name, .. } if *conversation == id && name == "Kalibermarkt")
        })
    });
    assert_eq!(projects.active(), None);
    runner.cancel_all();
}
