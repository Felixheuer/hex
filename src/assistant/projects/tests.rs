//! Projects: their folders, souls, the active one, what tasks get and
//! add, suggestions, spoken commands, and the agent's tool.

use super::command::{Command, parse};
use super::*;
use serde_json::json;

fn scratch() -> Projects {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let base = std::env::temp_dir().join(format!(
        "agent-desk-projects-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&base);
    Projects::at(base.join("Projects"), base.join("state").join("projects.json"))
}

#[test]
fn a_project_is_a_folder_with_a_soul_an_index_and_a_files_area() {
    let projects = scratch();
    assert!(projects.list().is_empty());
    assert_eq!(projects.active(), None, "no project until one is chosen");
    let (startup, made) = projects.create("  Startup ").unwrap();
    assert!(made);
    assert_eq!(startup.name, "Startup");
    assert_eq!(startup.folder, projects.root.join("Startup"));
    assert!(startup.files_folder().is_dir());
    let soul = projects.soul(&startup);
    assert!(soul.starts_with("# Startup\n"), "{soul}");
    for heading in ["## Goal", "## People", "## Websites", "## Repositories", "## Decisions", "## Recently learned"] {
        assert!(soul.contains(heading), "{heading} in {soul}");
    }
    let index: Index =
        serde_json::from_slice(&std::fs::read(startup.folder.join(INDEX)).unwrap()).unwrap();
    assert_eq!(index.id, startup.id);
    // The same name again is the same project.
    let (again, made) = projects.create("startup").unwrap();
    assert!(!made);
    assert_eq!(again.id, startup.id);
    // A folder the person made in Finder is adopted.
    std::fs::create_dir_all(projects.root.join("Privat")).unwrap();
    let names: Vec<String> = projects.list().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["Privat", "Startup"]);
    // Hidden folders aren't projects.
    std::fs::create_dir_all(projects.root.join(".Trash")).unwrap();
    assert_eq!(projects.list().len(), 2);
}

#[test]
fn the_active_project_is_remembered_with_the_recent_ones_first() {
    let projects = scratch();
    let (a, _) = projects.create("Alpha").unwrap();
    let (b, _) = projects.create("Beta").unwrap();
    let (c, _) = projects.create("Gamma").unwrap();
    projects.set_active(Some(&c.id)).unwrap();
    projects.set_active(Some(&a.id)).unwrap();
    assert_eq!(projects.active().unwrap().id, a.id);
    let order: Vec<String> = projects.list().into_iter().map(|p| p.name).collect();
    assert_eq!(order, ["Alpha", "Gamma", "Beta"], "recent first, then by name");
    projects.set_active(None).unwrap();
    assert_eq!(projects.active(), None);
    // Archived: out of the picker and no longer active.
    projects.set_active(Some(&b.id)).unwrap();
    projects.set_archived(&b.id, true).unwrap();
    assert_eq!(projects.active(), None);
    assert!(projects.list().iter().all(|p| p.id != b.id));
    assert!(projects.all().iter().any(|p| p.id == b.id && p.archived));
    assert!(projects.set_active(Some(&b.id)).is_err());
    // Making it again brings it back.
    let (back, made) = projects.create("Beta").unwrap();
    assert!(!made);
    assert_eq!(back.id, b.id);
    assert!(projects.list().iter().any(|p| p.id == b.id));
}

#[test]
fn renaming_moves_the_folder_and_the_soul_s_title() {
    let projects = scratch();
    let (project, _) = projects.create("Shop").unwrap();
    projects.set_active(Some(&project.id)).unwrap();
    let renamed = projects.rename(&project.id, "Kalibermarkt").unwrap();
    assert_eq!(renamed.id, project.id);
    assert_eq!(renamed.folder, projects.root.join("Kalibermarkt"));
    assert!(!project.folder.exists());
    assert!(projects.soul(&renamed).starts_with("# Kalibermarkt\n"));
    assert_eq!(projects.active().unwrap().name, "Kalibermarkt");
}

#[test]
fn a_spoken_name_finds_its_project() {
    let projects = scratch();
    projects.create("Kalibermarkt").unwrap();
    projects.create("Privat").unwrap();
    projects.create("Startup").unwrap();
    let found = |said: &str| projects.find(said).map(|p| p.name);
    assert_eq!(found("Startup").as_deref(), Some("Startup"));
    assert_eq!(found("startup").as_deref(), Some("Startup"));
    assert_eq!(found("Kaliber Markt").as_deref(), Some("Kalibermarkt"));
    assert_eq!(found("Kalibermark").as_deref(), Some("Kalibermarkt"));
    assert_eq!(found("Private").as_deref(), Some("Privat"));
    assert_eq!(found("Garten"), None);
    assert_eq!(found(""), None);
}

#[test]
fn the_soul_keeps_sections_dedupes_and_never_keeps_secrets() {
    let projects = scratch();
    let (project, _) = projects.create("Startup").unwrap();
    assert!(projects.add(&project, Part::Goal, "Seed round by March").unwrap());
    assert!(!projects.add(&project, Part::Goal, "- seed round by march").unwrap());
    assert!(projects.add(&project, Part::People, "Anna Schmidt, CTO, anna@startup.io").unwrap());
    assert!(projects.add(&project, Part::Learned, "pricing page draft").is_ok());
    let soul = projects.soul(&project);
    let goal = soul.find("## Goal").unwrap();
    let people = soul.find("## People").unwrap();
    let seed = soul.find("- Seed round by March").unwrap();
    assert!(goal < seed && seed < people, "{soul}");
    // A person's own section survives a rewrite.
    let mut text = soul.clone();
    text.push_str("\n## Investors\n\n- Hans Berger\n");
    projects.write_soul(&project, &text).unwrap();
    projects.add(&project, Part::Decisions, "Rust for the backend").unwrap();
    let soul = projects.soul(&project);
    assert!(soul.contains("## Investors\n\n- Hans Berger"), "{soul}");
    // Secrets are refused.
    for secret in ["The admin password is hunter2", "API key sk-abc123"] {
        assert!(projects.add(&project, Part::Accounts, secret).is_err());
        assert!(projects.learn(&project, Part::Accounts, secret).is_err());
    }
    assert!(!projects.soul(&project).contains("hunter2"));
}

#[test]
fn what_was_learned_once_never_comes_back_after_it_was_pruned() {
    let projects = scratch();
    let (project, _) = projects.create("Startup").unwrap();
    assert!(projects.learn(&project, Part::Sites, "startup.io").unwrap());
    assert!(!projects.learn(&project, Part::Sites, "https://www.startup.io").unwrap());
    let soul = projects.soul(&project);
    assert!(soul.contains("## Recently learned\n\n- Websites: startup.io"), "{soul}");
    // The person deletes the line; it isn't added again.
    projects
        .write_soul(&project, &soul.replace("- Websites: startup.io\n", ""))
        .unwrap();
    assert!(!projects.learn(&project, Part::Sites, "startup.io").unwrap());
    assert!(!projects.soul(&project).contains("startup.io"));
    // What the soul already says isn't learned again.
    projects.add(&project, Part::Repos, "github.com/acme/app").unwrap();
    assert!(!projects.learn(&project, Part::Repos, "github.com/acme/app").unwrap());
}

#[test]
fn recently_learned_keeps_its_newest_lines() {
    let mut soul = soul::parse(&soul::template("X"));
    for n in 0..soul::LEARNED_MAX + 5 {
        assert!(soul.add(Part::Learned, &format!("fact {n}")));
    }
    let learned = soul.section(Part::Learned).unwrap();
    let items: Vec<&String> = learned.lines.iter().filter(|line| line.starts_with("- ")).collect();
    assert_eq!(items.len(), soul::LEARNED_MAX);
    assert_eq!(items[0], "- fact 5");
    assert_eq!(items.last().unwrap().as_str(), format!("- fact {}", soul::LEARNED_MAX + 4));
}

#[test]
fn a_task_in_a_project_gets_its_soul_bounded_and_the_relevant_part_first() {
    let projects = scratch();
    let (project, _) = projects.create("Startup").unwrap();
    projects.add(&project, Part::Goal, "Ship the beta in May").unwrap();
    projects.add(&project, Part::People, "Max Weber, designer, max@startup.io").unwrap();
    for n in 0..200 {
        projects
            .add(&project, Part::Decisions, &format!("Decision number {n} about the roadmap and the colours"))
            .unwrap();
    }
    projects.add(&project, Part::Sites, "pricing: startup.io/pricing").unwrap();
    let task = projects.for_task(&project, "Update the pricing page");
    assert_eq!(task.id, project.id);
    assert_eq!(task.jobs, project.folder.join(JOBS));
    assert!(task.context.starts_with("This request belongs to the person's project “Startup”"));
    assert!(task.context.contains("- pricing: startup.io/pricing"), "the part the request names");
    assert!(task.context.contains("- Ship the beta in May"));
    assert!(task.context.contains(&project.files_folder().display().to_string()));
    assert!(task.context.contains("hex_project"));
    assert!(task.context.chars().count() < BRIEF_CHARS + 900, "bounded");
    // Sections stay in the file's order under their headings.
    let goal = task.context.find("## Goal").unwrap();
    let sites = task.context.find("## Websites").unwrap();
    assert!(goal < sites);
    // An empty soul says so.
    let (empty, _) = projects.create("Leer").unwrap();
    assert!(projects.for_task(&empty, "x").context.contains("(nothing written yet)"));
}

#[test]
fn a_finished_task_adds_what_belongs_to_the_project_and_no_secrets() {
    let projects = scratch();
    let (project, _) = projects.create("Kalibermarkt").unwrap();
    let workspace = project.jobs_folder().join("2026-11-13 Preise");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("Preise.xlsx"), b"x").unwrap();
    std::fs::write(workspace.join("plan.md"), b"- [x] a").unwrap();
    std::fs::write(workspace.join("report.md"), b"done").unwrap();
    let added = projects.collect(
        &project,
        "Vergleiche die Preise auf kalibermarkt.de mit github.com/kaliber/shop, schreib an lea@kalibermarkt.de",
        "Done: staging.kalibermarkt.de is up; I read example.com and wikipedia.org. Password: hunter2",
        Some(&workspace),
    );
    assert!(added.contains(&"kalibermarkt.de".to_owned()), "{added:?}");
    assert!(added.contains(&"github.com/kaliber/shop".to_owned()), "{added:?}");
    assert!(added.contains(&"lea@kalibermarkt.de".to_owned()), "{added:?}");
    assert!(added.contains(&"staging.kalibermarkt.de".to_owned()), "carries the project's name");
    assert!(!added.iter().any(|value| value.contains("example.com") || value.contains("wikipedia")), "only what is the project's: {added:?}");
    assert!(added.iter().any(|value| value.ends_with("Preise.xlsx")), "{added:?}");
    assert!(!added.iter().any(|value| value.ends_with("plan.md") || value.ends_with("report.md")));
    let soul = projects.soul(&project);
    assert!(!soul.contains("hunter2"));
    assert!(soul.contains("- Key files: Jobs/2026-11-13 Preise/Preise.xlsx"), "{soul}");
    // Again: nothing new.
    assert!(projects
        .collect(&project, "kalibermarkt.de", "staging.kalibermarkt.de", Some(&workspace))
        .is_empty());
}

#[test]
fn a_request_naming_a_project_s_site_repo_person_or_name_suggests_it() {
    let projects = scratch();
    let (shop, _) = projects.create("Kalibermarkt").unwrap();
    projects.add(&shop, Part::Sites, "kalibermarkt.de").unwrap();
    let (startup, _) = projects.create("Startup").unwrap();
    projects.add(&startup, Part::People, "Anna Schmidt, CTO").unwrap();
    projects.add(&startup, Part::Repos, "github.com/acme/rocket-app").unwrap();
    let suggest = |said: &str| projects.suggest(said).map(|p| p.name);
    assert_eq!(suggest("Wie viele Besucher hatte kalibermarkt.de gestern?").as_deref(), Some("Kalibermarkt"));
    assert_eq!(suggest("Schreib Anna Schmidt wegen dem Termin").as_deref(), Some("Startup"));
    assert_eq!(suggest("Open a PR on rocket-app").as_deref(), Some("Startup"));
    assert_eq!(suggest("How is the startup doing?").as_deref(), Some("Startup"));
    assert_eq!(suggest("Wie wird das Wetter morgen?"), None);
    assert_eq!(suggest("Anna hat angerufen"), None, "a first name alone isn't enough");
    // Two named as surely: no guess.
    projects.create("Privat").unwrap();
    assert_eq!(suggest("Startup oder Privat?"), None);
}

#[test]
fn spoken_project_commands_are_understood_in_german_and_english() {
    let switch = |name: &str| Some(Command::Switch(name.into()));
    let create = |name: &str| Some(Command::Create(name.into()));
    for (said, wanted) in [
        ("Wechsle zu Projekt Startup", switch("Startup")),
        ("wechsel zum Projekt Kalibermarkt.", switch("Kalibermarkt")),
        ("Bitte wechsle ins Projekt „Privat“", switch("Privat")),
        ("Wechsle das Projekt zu Startup", switch("Startup")),
        ("Öffne Projekt Startup", switch("Startup")),
        ("Projekt Startup öffnen", switch("Startup")),
        ("Arbeite jetzt im Projekt Startup", switch("Startup")),
        ("Neues Projekt Kalibermarkt", create("Kalibermarkt")),
        ("Erstelle ein neues Projekt namens Garten", create("Garten")),
        ("Leg ein neues Projekt Steuer 2026 an", create("Steuer 2026")),
        ("Ohne Projekt", Some(Command::Leave)),
        ("kein Projekt mehr", Some(Command::Leave)),
        ("Projekt verlassen", Some(Command::Leave)),
        ("Welches Projekt ist aktiv?", Some(Command::Which)),
        ("In welchem Projekt bin ich?", Some(Command::Which)),
        ("Switch to project Startup", switch("Startup")),
        ("switch to the Startup project", switch("Startup")),
        ("Open project Garden", switch("Garden")),
        ("New project Kalibermarkt", create("Kalibermarkt")),
        ("Create a new project called Garden", create("Garden")),
        ("No project", Some(Command::Leave)),
        ("Leave the project, please", Some(Command::Leave)),
        ("Which project?", Some(Command::Which)),
    ] {
        assert_eq!(parse(said), wanted, "{said}");
    }
    for request in [
        "Mach eine Präsentation für das Projekt Startup",
        "Wie läuft das Projekt?",
        "Schreib Anna wegen dem neuen Projekt und dem Termin morgen um zehn",
        "What's the status of the project?",
        "Open Safari",
        "",
    ] {
        assert_eq!(parse(request), None, "{request}");
    }
}

#[test]
fn the_agent_s_tool_reads_learns_and_only_the_person_s_turn_switches() {
    let projects = scratch();
    let (project, _) = projects.create("Startup").unwrap();
    let call = |job: bool, arguments: serde_json::Value| {
        tool::call(&projects, Some(&project.id), job, &arguments)
    };
    let shown = call(true, json!({"action": "show"})).unwrap();
    assert!(shown.contains("Project “Startup”") && shown.contains("## Goal"), "{shown}");
    assert!(call(true, json!({"action": "learn", "section": "sites", "text": "startup.io"}))
        .unwrap()
        .contains("Added"));
    assert!(call(true, json!({"action": "learn", "text": "my password is x"})).is_err());
    let refused = call(true, json!({"action": "switch", "project": "none"})).unwrap_err();
    assert!(refused.contains("background job"), "{refused}");
    assert!(call(true, json!({"action": "update", "section": "goal", "text": "x"})).is_err());
    assert!(call(false, json!({"action": "update", "section": "goal", "text": "Profitable in 2027"}))
        .unwrap()
        .contains("Added"));
    assert!(projects.soul(&project).contains("## Goal\n\n- Profitable in 2027"));
    call(false, json!({"action": "create", "project": "Privat"})).unwrap();
    call(false, json!({"action": "switch", "project": "privat"})).unwrap();
    assert_eq!(projects.active().unwrap().name, "Privat");
    let listed = call(false, json!({"action": "list"})).unwrap();
    assert!(listed.contains("Privat [active]"), "{listed}");
    call(false, json!({"action": "switch", "project": "none"})).unwrap();
    assert_eq!(projects.active(), None);
    // A task in no project names one.
    let none = tool::call(&projects, None, false, &json!({"action": "show"})).unwrap_err();
    assert!(none.contains("isn't in a project"), "{none}");
}

#[test]
fn memory_learned_in_a_project_is_tagged_and_stays_out_of_other_projects() {
    use crate::assistant::knowledge::graph::{Graph, extract};
    let mut graph = Graph::default();
    graph.learn(&extract("Max Weber's email is max@weber.de", None, "DE"), "remember", 1);
    graph.scope = Some("startup".into());
    graph.learn(&extract("Max Weber works at Rocket GmbH", None, "DE"), "remember", 2);
    let tagged: Vec<Option<&str>> = graph.links.iter().map(|link| link.project.as_deref()).collect();
    assert_eq!(tagged, [None, Some("startup")], "global stays global");
    let in_startup = graph.context("Schreib Max Weber").unwrap();
    assert!(in_startup.contains("max@weber.de") && in_startup.contains("Rocket GmbH"), "{in_startup}");
    graph.scope = Some("privat".into());
    let elsewhere = graph.context("Schreib Max Weber").unwrap();
    assert!(elsewhere.contains("max@weber.de"), "{elsewhere}");
    assert!(!elsewhere.contains("Rocket"), "another project's link stays out: {elsewhere}");
    graph.scope = None;
    assert!(graph.context("Schreib Max Weber").unwrap().contains("Rocket GmbH"), "no project: everything, as before");
    // The tag survives the file; the scope doesn't.
    let saved: Graph = serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
    assert_eq!(saved.links[1].project.as_deref(), Some("startup"));
    assert_eq!(saved.scope, None);
}
