//! Prompts, lanes, and the guard's view of a task.

use super::*;
use crate::assistant::types::{Context, TaskWindow};

fn job(text: &str, origin: Origin, context: Context) -> Job {
    Job {
        conversation: "c1".into(),
        request: Request {
            conversation: None,
            text: text.into(),
            origin,
            context,
        },
        settings: AssistantSettings::default(),
        agent_id: "hex-7".into(),
        desktop_session: Some("S1".into()),
        kind: JobKind::Ask,
        digest: None,
        workspace: None,
        authorized: Default::default(),
        memory: None,
        helps: None,
    }
}

fn mail() -> TaskWindow {
    TaskWindow {
        app: "Mail".into(),
        window_id: Some("w-3".into()),
    }
}

#[test]
fn a_quick_question_carries_what_was_on_screen() {
    let context = Context {
        screenshot: None,
        selected_text: Some(" the invoice total ".into()),
        frontmost_app: Some("Mail".into()),
        window_title: Some("Inbox".into()),
        window: Some(mail()),
        display: Some("The person is working on M27Q (display 5).".into()),
    };
    let prompt = prompt(
        &job("What's this?", Origin::ActionShortcut, context),
        &[],
        false,
    );
    assert_eq!(
        prompt,
        "What the person has on screen as they ask:\nFrontmost app: Mail\nIts window: Inbox\nYour only window, the one in front: Mail (window id w-3)\nDisplays: The person is working on M27Q (display 5).\nSelected text:\n<<<\nthe invoice total\n>>>\n\nThe person asks: What's this?"
    );
    let plain = job("Hi", Origin::ActionShortcut, Context::default());
    assert_eq!(super::prompt(&plain, &[], true), "Hi");
}

#[test]
fn a_new_session_is_told_the_conversation_so_far() {
    let history = [
        Message {
            role: Role::User,
            parts: vec![Part::Text {
                text: "2+2?".into(),
            }],
            at_ms: 1,
        },
        Message {
            role: Role::Assistant,
            parts: vec![Part::Text { text: "4".into() }],
            at_ms: 2,
        },
    ];
    let mut job = job("And times 3?", Origin::ActionShortcut, Context::default());
    job.digest = Some("Other tasks: none".into());
    let prompt = prompt(&job, &history, true);
    // The digest is for a new conversation only.
    assert_eq!(
        prompt,
        "The conversation so far:\nPerson: 2+2?\nYou: 4\n\nThe person asks: And times 3?"
    );
}

#[test]
fn a_new_conversation_starts_with_what_else_is_going_on() {
    let mut job = job(
        "mach das auch für die andere Mail",
        Origin::Background,
        Context::default(),
    );
    job.digest = Some("Running now:\n- Reply to Anna (Mail): Typing".into());
    assert_eq!(
        prompt(&job, &[], true),
        "Running now:\n- Reply to Anna (Mail): Typing\n\nThe person asks: mach das auch für die andere Mail"
    );
}

#[test]
fn steering_restarts_and_resumes_say_what_happened() {
    let mut steer = job("nimm lieber Anna", Origin::Background, Context::default());
    steer.kind = JobKind::Steer {
        doing: Some("Typing 12 characters into “To”".into()),
        steps: vec!["Opened Mail".into(), "Clicked “New Message”".into()],
    };
    assert_eq!(
        prompt(&steer, &[], false),
        "The person interrupted you while you were doing this: Typing 12 characters into “To” (last steps: Opened Mail; Clicked “New Message”). Their new instruction: “nimm lieber Anna”. Adjust what you are doing; don't redo finished steps."
    );
    let mut resume = job("Schreib Anna", Origin::Background, Context::default());
    resume.kind = JobKind::Resume;
    let text = prompt(&resume, &[], false);
    assert!(text.starts_with(RESUME_NOTE), "{text}");
    assert!(text.ends_with("The task was: “Schreib Anna”"));
    resume.kind = JobKind::AfterLimit;
    assert!(prompt(&resume, &[], false).starts_with(LIMIT_NOTE));
}

#[test]
fn the_guard_learns_each_lanes_window_and_input_policy() {
    let context = Context {
        window: Some(mail()),
        ..Context::default()
    };
    let mut here = job("What's this?", Origin::ActionShortcut, context.clone());
    here.settings.here_physical_input = true;
    here.settings.physical_input = false;
    here.settings.blocked_apps = vec!["1Password".into()];
    let spec = task_spec(&here);
    assert_eq!(
        spec,
        TaskSpec {
            task: "c1".into(),
            lane: Lane::Here,
            agent_id: "hex-7".into(),
            session: Some("S1".into()),
            here_window: Some(mail()),
            physical_input: true,
            blocked_apps: vec!["1Password".into()],
            // Full control is the default (`background_front_window`).
            full_control: true,
        }
    );
    let mut background = here.clone();
    background.request.origin = Origin::Chat;
    let spec = task_spec(&background);
    assert_eq!(spec.lane, Lane::Background);
    assert_eq!(spec.here_window, None);
    assert!(!spec.physical_input);
    // Asked to stay in the background, a task leaves the person's windows.
    background.request.text = "Sortier das, aber arbeite nur im Hintergrund".into();
    assert!(!task_spec(&background).full_control);
}

#[test]
fn the_here_lane_runs_on_the_fast_model_where_the_backend_takes_it() {
    let mut here = job("q", Origin::ActionShortcut, Context::default());
    here.settings.claude_code_model = Some("opus".into());
    assert_eq!(model(&here).as_deref(), Some("claude-haiku-5-5"));
    let mut background = here.clone();
    background.request.origin = Origin::Background;
    assert_eq!(model(&background).as_deref(), Some("opus"));
    here.settings.backend = AssistantBackend::Codex;
    here.settings.codex_model = Some("gpt-x".into());
    assert_eq!(model(&here).as_deref(), Some("gpt-x"));
    here.settings.backend = AssistantBackend::ClaudeApi;
    here.settings.here_model = None;
    assert_eq!(model(&here).as_deref(), Some("claude-sonnet-5-5"));
}

#[test]
fn each_lane_gets_its_own_rules() {
    let desktop = Desktop::fixture(Some("S1"), "hex-7");
    let here = system_prompt(Origin::ActionShortcut, Some(&desktop), false);
    assert!(here.contains("ACT only in the window in front"));
    assert!(here.contains("speakable"));
    assert!(!here.contains("desktop_launch"));
    let background = system_prompt(Origin::Background, Some(&desktop), true);
    assert!(background.contains("open apps with hex_open_app"));
    assert!(background.contains("desktop_claim_window"));
    assert!(background.contains("The clipboard is Here-only"));
    assert!(background.contains("Never the person's window"));
    assert!(background.contains("Use the browser tools for anything on the web"));
    assert!(background.contains("agent “hex-7”"));
    let quick = system_prompt(Origin::ActionShortcut, None, false);
    assert!(quick.contains("no markdown"));
    assert!(quick.contains("no access to the desktop"));
    let chat = system_prompt(Origin::Chat, None, false);
    assert!(chat.contains("Markdown is fine"));
}

#[test]
fn the_prompt_explains_the_autonomy_mode_and_memory() {
    use crate::assistant::autonomy::{Authorized, Autonomy, Category};
    let share = Authorized {
        categories: [Category::Share].into(),
        notes: vec!["only to max@example.com".into()],
        ..Authorized::default()
    };
    let smart = autonomy_rules(Autonomy::Smart, &share);
    assert!(smart.contains("you may only share without asking because the person asked for it (only to max@example.com)"), "{smart}");
    assert!(smart.contains("HEX_CONFIRM: <category> <target>"));
    let nothing = autonomy_rules(Autonomy::Smart, &Authorized::default());
    assert!(nothing.contains("prepare (draft, save, fill in), nothing consequential"));
    assert!(autonomy_rules(Autonomy::Ask, &Authorized::default()).contains("every send, delete"));
    let yolo = autonomy_rules(Autonomy::Yolo, &share);
    assert!(
        yolo.contains("never ask for permission") && !yolo.contains("drafts"),
        "{yolo}"
    );
    let workspace = std::path::Path::new("/tmp/job");
    let free = job_rules(workspace, true, true);
    assert!(
        free.contains("without a sandbox") && free.contains("## Done when"),
        "{free}"
    );
    assert!(free.contains("open -na Ghostty"));
    let kept = job_rules(workspace, true, false);
    assert!(
        kept.contains("Shell commands run sandboxed") && kept.contains("save emails as drafts")
    );
    let memory = memory_rules(Some("- Company: blabla.com"), true);
    assert!(memory.contains("hex_remember"));
    assert!(memory.contains("<<<\n- Company: blabla.com\n>>>"));
    // A job reads the note but can't add to it.
    let read_only = memory_rules(Some("- Company: blabla.com"), false);
    assert!(!read_only.contains("hex_remember"));
    assert!(read_only.contains("- Company: blabla.com"));
    assert!(kept.contains("hex_team_start"), "a job may lead a team");
    let mut confirm = job("Ja, schick sie", Origin::Background, Context::default());
    confirm.kind = JobKind::Confirm {
        question: "Shall I send the 3 emails?".into(),
        allowed: true,
    };
    assert!(prompt(&confirm, &[], false).contains("That's a yes"));
}

#[test]
fn front_matter_is_dropped_from_the_guide() {
    assert_eq!(
        strip_front_matter("---\nname: x\n---\n\n# Guide"),
        "# Guide"
    );
    assert_eq!(strip_front_matter("# Guide"), "# Guide");
}

#[test]
fn a_session_s_rules_fingerprint_ignores_the_request_s_permissions() {
    let base = "You are Hex's assistant.\nRules: work in the window.\n";
    let a = format!("{base}\nAutonomy (Smart): you may only send because…\nmemory A");
    let b = format!("{base}\nAutonomy (Yolo): nothing waits…\nmemory B");
    assert_eq!(super::rules_fingerprint(&a), super::rules_fingerprint(&b));
    let changed = format!(
        "{base}- If the request needs anything beyond that window, hand it off.\n\nAutonomy (Smart): …"
    );
    assert_ne!(
        super::rules_fingerprint(&a),
        super::rules_fingerprint(&changed)
    );
}

#[test]
fn browsers_start_with_the_person_s_accounts_and_a_clean_one_is_asked_for() {
    let prompt = system_prompt(Origin::Background, None, true);
    assert!(prompt.contains("start with the person's accounts"));
    assert!(prompt.contains("`signedIn: false` for a clean one"));
    assert!(prompt.contains("Never ask the person to sign in unless a page still shows a login"));
}

fn said(text: &str, at_ms: u64) -> crate::assistant::types::Message {
    crate::assistant::types::Message {
        role: Role::User,
        parts: vec![crate::assistant::types::Part::Text { text: text.into() }],
        at_ms,
    }
}

#[test]
fn an_escalated_job_never_adds_its_request_again() {
    let mut stored = Stored::new(
        "c1",
        Origin::Chat,
        "Teste dich selber und notiere die Lücken.",
    );
    stored.push(said("Teste dich selber und notiere die Lücken.", 1));
    // The person wrote more before the job started.
    stored.push(said("Wie weit bist du?", 2));
    assert!(super::said_before(
        &stored,
        "Teste dich selber und notiere die Lücken."
    ));
    // Or the more was added to its request while it waited.
    assert!(super::said_before(
        &stored,
        "Teste dich selber und notiere die Lücken.\n\nnimm Freitag"
    ));
    // A new job in a new conversation adds it; a short earlier word
    // ("Ja") is no prefix.
    let mut fresh = Stored::new("c2", Origin::Background, "Ja, recherchiere das");
    assert!(!super::said_before(&fresh, "Ja, recherchiere das"));
    fresh.push(said("Ja", 1));
    assert!(!super::said_before(&fresh, "Ja, recherchiere das"));
}

#[test]
fn a_steer_written_when_said_is_not_added_again() {
    let mut stored = Stored::new("c1", Origin::Background, "job");
    stored.push(said("use Friday instead", 1_000));
    assert!(super::said_lately(
        &stored,
        "use Friday instead",
        1_000 + 5_000
    ));
    assert!(!super::said_lately(
        &stored,
        "use Friday instead",
        1_000 + 11 * 60_000
    ));
    assert!(!super::said_lately(&stored, "something else", 2_000));
}

#[test]
fn a_hand_off_starts_its_jobs_and_a_mere_claim_starts_one() {
    let asked = job(
        "Kannst du auch ein Werbevideo über diese Webseite machen?",
        Origin::ActionShortcut,
        Context::default(),
    );
    // The tool's jobs: the first carries its own part when there are several.
    let handoff = handoff::Handoff {
        jobs: vec![
            handoff::Job {
                title: "Werbevideo".into(),
                task: "Mach ein Werbevideo über example.com".into(),
            },
            handoff::Job {
                title: "Wettbewerber".into(),
                task: "Recherchiere drei Wettbewerber".into(),
            },
        ],
        say: Some("Mach ich: zwei Aufträge.".into()),
    };
    match super::handed_on(&asked, "Mach ich.", Some(handoff)) {
        Some(Outcome::Escalate {
            title,
            reply,
            more,
            first_task,
        }) => {
            assert_eq!(title, "Werbevideo");
            assert_eq!(reply, "Mach ich: zwei Aufträge.");
            assert_eq!(more.len(), 1);
            assert_eq!(
                first_task.as_deref(),
                Some("Mach ein Werbevideo über example.com")
            );
        }
        _ => panic!("the hand-off should escalate"),
    }
    // The pilot.14 bug: prose that claims the background, no tool, no marker.
    match super::handed_on(
        &asked,
        "Das läuft als Hintergrundauftrag, und das Video landet im Arbeitsordner.",
        None,
    ) {
        Some(Outcome::Escalate {
            title, first_task, ..
        }) => {
            assert!(title.starts_with("Kannst du auch ein Werbevideo"));
            assert_eq!(first_task, None, "the whole request is the job");
        }
        _ => panic!("a claimed background job should start"),
    }
    // A plain answer starts nothing.
    assert!(super::handed_on(&asked, "Ja, das kann ich.", None).is_none());
}

#[test]
fn the_routing_rule_hands_jobs_on_with_the_tool() {
    assert!(ROUTING.contains("hex_start_jobs"));
    assert!(
        ROUTING.contains("is ONE job"),
        "dependent parts stay one job"
    );
    let here = system_prompt(
        Origin::ActionShortcut,
        Some(&Desktop::fixture(None, "hex-1")),
        false,
    );
    assert!(here.contains("hex_start_jobs"));
}
