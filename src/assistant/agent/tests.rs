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
    };
    let prompt = prompt(
        &job("What's this?", Origin::ActionShortcut, context),
        &[],
        false,
    );
    assert_eq!(
        prompt,
        "What the person has on screen as they ask:\nFrontmost app: Mail\nIts window: Inbox\nYour only window, the one in front: Mail (window id w-3)\nSelected text:\n<<<\nthe invoice total\n>>>\n\nThe person asks: What's this?"
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
        }
    );
    let mut background = here.clone();
    background.request.origin = Origin::Chat;
    let spec = task_spec(&background);
    assert_eq!(spec.lane, Lane::Background);
    assert_eq!(spec.here_window, None);
    assert!(!spec.physical_input);
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
    assert!(here.contains("Work only in the window in front"));
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
    assert!(autonomy_rules(Autonomy::Yolo, &share).contains("drafts unless"));
    let memory = memory_rules(Some("- Company: blabla.com"));
    assert!(memory.contains("hex_remember"));
    assert!(memory.contains("<<<\n- Company: blabla.com\n>>>"));
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
