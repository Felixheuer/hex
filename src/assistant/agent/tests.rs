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
        recall: None,
        helps: None,
    }
}

/// A job's tools on Claude Code with the desktop and the browser.
fn all_tools() -> JobTools {
    JobTools {
        backend: AssistantBackend::ClaudeCode,
        desktop: true,
        browser: true,
        team: true,
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
        "What the person has on screen as they ask:\nFrontmost app: Mail\nIts window: Inbox\nThe window in front: Mail (window id w-3)\nDisplays: The person is working on M27Q (display 5).\nSelected text:\n<<<\nthe invoice total\n>>>\n\nThe person asks: What's this?"
    );
    let plain = job("Hi", Origin::ActionShortcut, Context::default());
    assert_eq!(super::prompt(&plain, &[], true), "Hi");
    // Without full control the window in front is its only one.
    let mut kept = job(
        "What's this?",
        Origin::ActionShortcut,
        Context {
            window: Some(mail()),
            ..Context::default()
        },
    );
    kept.settings.full_control = false;
    assert!(
        super::prompt(&kept, &[], false)
            .contains("Your only window, the one in front: Mail (window id w-3)")
    );
}

#[test]
fn every_turn_says_what_runs_now_before_the_question() {
    let mut asked = job(
        "Kannst du mal WhatsApp öffnen?",
        Origin::ActionShortcut,
        Context {
            frontmost_app: Some("Mail".into()),
            ..Context::default()
        },
    );
    asked.digest = Some("Finished recently: WhatsApp is open on another desktop".into());
    let now = "The desktop right now (…):\nRunning apps:\n- WhatsApp: no open window";
    let prompt = super::prompt_with(&asked, &[], false, Some(now));
    let digest = prompt.find("Finished recently").unwrap();
    let desktop = prompt.find("The desktop right now").unwrap();
    let screen = prompt.find("What the person has on screen").unwrap();
    let question = prompt.find("The person asks: Kannst du").unwrap();
    // The fresh look comes after the older stream, so it is what stands.
    assert!(
        digest < desktop && desktop < screen && screen < question,
        "{prompt}"
    );
    // A steer, an answer, or a follow-up gets it too.
    asked.kind = JobKind::Answer {
        question: "Which chat?".into(),
    };
    assert!(
        super::prompt_with(&asked, &[], false, Some(now)).contains("- WhatsApp: no open window")
    );
}

#[test]
fn staying_in_the_background_still_opens_apps_in_windows_of_its_own() {
    let asked = job(
        "Kannst du jede beliebige App nur im Hintergrund bedienen?",
        Origin::ActionShortcut,
        Context::default(),
    );
    let spec = task_spec(&asked);
    assert!(!spec.full_control);
    assert!(spec.open_apps);
    let mut desktop = Desktop::fixture(None, "hex-1");
    desktop.open_apps = true;
    let here = system_prompt(Origin::ActionShortcut, Some(&desktop), false);
    assert!(
        here.contains("open or switch to any app with hex_open_app"),
        "{here}"
    );
    assert!(here.contains("their own windows left alone"));
}

#[test]
fn a_running_job_is_told_the_persons_own_words() {
    let asked = job(
        "Kannst du auch WhatsApp im Hintergrund öffnen?",
        Origin::ActionShortcut,
        Context::default(),
    );
    assert_eq!(
        super::told_words(
            &asked,
            "Der Nutzer fragt: Bitte öffne WhatsApp im Hintergrund."
        ),
        "Kannst du auch WhatsApp im Hintergrund öffnen?"
    );
    // A job telling its helpers passes its own words.
    let mut lead = job("Recherchiere", Origin::Background, Context::default());
    lead.workspace = Some("/tmp/job".into());
    assert_eq!(super::told_words(&lead, "Nimm Firma B"), "Nimm Firma B");
}

#[test]
fn small_things_and_capability_questions_never_become_jobs() {
    let routing = routing(true, true, true);
    assert!(routing.contains("Never a job: opening, switching to, or using one app"));
    assert!(routing.contains("can you …?"));
    assert!(routing.contains("pass it to that job with hex_tell_job instead of starting another"));
    let rules = job_rules(std::path::Path::new("/tmp/job"), all_tools(), true);
    assert!(rules.contains("skips the paperwork"), "{rules}");
    assert!(rules.contains("Only when the job sent, posted, published"));
    let background = system_prompt(
        Origin::Background,
        Some(&Desktop::fixture(None, "hex-1")),
        true,
    );
    assert!(background.contains("even one that already runs, call hex_open_app first"));
    assert!(background.contains("Never only check that an app runs"));
}

#[test]
fn with_full_control_a_quick_turn_opens_apps_itself() {
    let mut desktop = Desktop::fixture(Some("S1"), "hex-7");
    desktop.front_window = true;
    let here = system_prompt(Origin::ActionShortcut, Some(&desktop), false);
    assert!(
        here.contains("open or switch to any app with hex_open_app"),
        "{here}"
    );
    assert!(here.contains("Never say you can't open, reach, or use an app"));
    assert!(here.contains("never offer a background job for something you can do now"));
    assert!(
        here.contains("ask what to do in it"),
        "an app opened without a task asks"
    );
    assert!(here.contains("trust it over anything said earlier"));
    assert!(!here.contains("ACT only in the window in front"));
    // Routing no longer sends every other app to the background.
    let routing = routing(true, true, false);
    assert!(!routing.contains("other apps,"));
    assert!(routing.contains("open or switch to it with hex_open_app"));
    // Kept to the window in front, apps are a job, and no hex_open_app.
    let kept = super::routing(true, false, false);
    assert!(kept.contains("other apps,"), "{kept}");
    assert!(!kept.contains("hex_open_app"), "{kept}");
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
    here.settings.background_physical_input = false;
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
            // Full control is the default (`full_control`).
            full_control: true,
            open_apps: true,
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
    assert_eq!(model(&here).as_deref(), Some("claude-sonnet-5-5"));
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
    assert!(background.contains("to use any app, even one that already runs, call hex_open_app"));
    assert!(background.contains("desktop_claim_window"));
    assert!(
        !background.contains("The clipboard is Here-only"),
        "the guard offers it where allowed"
    );
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
    let free = job_rules(workspace, all_tools(), true);
    assert!(
        free.contains("without a sandbox") && free.contains("## Done when"),
        "{free}"
    );
    assert!(free.contains("open -na Ghostty"));
    let kept = job_rules(workspace, all_tools(), false);
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
fn a_session_s_rules_fingerprint_counts_every_rule_but_the_memory_note() {
    use crate::assistant::autonomy::{Authorized, Category};
    let run = Run::fixture(AssistantBackend::ClaudeCode);
    let fingerprint = |job: &Job| super::rules_fingerprint(job, &run);
    let mut a = job("Send it to Max", Origin::ActionShortcut, Context::default());
    a.settings.memory = true;
    a.memory = Some("- Company: A".into());
    // Another note, other words, and what they allowed: the same rules.
    let mut b = a.clone();
    b.memory = Some("- Company: A\n- Max: max@example.com".into());
    b.request.text = "Schreib Max".into();
    b.authorized = Authorized {
        categories: [Category::Send].into(),
        ..Authorized::default()
    };
    b.recall = Some("- Max (person)".into());
    assert_eq!(fingerprint(&a), fingerprint(&b));
    // Rules after the autonomy section count: the memory rules, the tools'
    // rules (a job has none), and the mode.
    let mut no_memory = a.clone();
    no_memory.settings.memory = false;
    assert_ne!(fingerprint(&a), fingerprint(&no_memory));
    let mut background = a.clone();
    background.request.origin = Origin::Background;
    assert_ne!(fingerprint(&a), fingerprint(&background));
    let mut yolo = a.clone();
    yolo.settings.autonomy = crate::assistant::autonomy::Autonomy::Yolo;
    assert_ne!(fingerprint(&a), fingerprint(&yolo));
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
        tells: Vec::new(),
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
    for (desktop, reach, browser) in [
        (true, true, true),
        (true, false, false),
        (false, false, true),
    ] {
        let routing = routing(desktop, reach, browser);
        assert!(routing.contains("call the hex_start_jobs tool"));
        assert!(
            routing.contains("is ONE job"),
            "dependent parts stay one job"
        );
    }
    let here = system_prompt(
        Origin::ActionShortcut,
        Some(&Desktop::fixture(None, "hex-1")),
        false,
    );
    assert!(here.contains("hex_start_jobs"));
}

#[test]
fn an_unattended_job_asks_everything_up_front_and_proves_its_work() {
    let rules = job_rules(std::path::Path::new("/tmp/job"), all_tools(), true);
    assert!(rules.contains("ask ALL of it in ONE message"), "{rules}");
    assert!(rules.contains("never ask for something you could have asked now"));
    assert!(rules.contains("Nachweis.md"), "the evidence file is named");
    assert!(
        rules.contains("SCREEN_LOCKED"),
        "a locked screen is explained"
    );
    let projects = project_rules("~/dev");
    assert!(projects.contains("under ~/dev"), "{projects}");
    assert!(projects.contains("--private") && projects.contains("unless the person said public"));
    assert!(projects.contains("Visual Studio Code"));
    assert!(projects.contains("placeholder images"));
    let apps = apps_rules(true, true);
    assert!(apps.contains("screenshot the sent message"));
    assert!(apps.contains("1440×900") && apps.contains("390×844"));
    assert!(apps.contains("a QR code or login form means not"));
    // Without the desktop, no desktop tools and no desktop errors.
    let web_only = apps_rules(false, true);
    assert!(!web_only.contains("hex_installed_apps") && !web_only.contains("SCREEN_LOCKED"));
}

/// The real failure: "Was ist das Passwort vom IP-Zugang von
/// Kraftstoffnotdienst? Steht in Notizen": the agent tried the Notes
/// window through Accessibility, couldn't reach it, and gave up. Every lane
/// that may open apps now learns, for any app, to script it first (read
/// the note without its window), and never to give up on an unreachable
/// window or send the person to fetch it.
#[test]
fn any_app_is_scripted_first_and_an_unreachable_window_means_another_way() {
    let rule = APP_METHODS;
    let script = rule.find("(1) its scripting").unwrap();
    let url = rule.find("(2) a URL scheme").unwrap();
    let ax = rule.find("(3) Accessibility").unwrap();
    let ocr = rule.find("(4) hex_read_window").unwrap();
    assert!(script < url && url < ax && ax < ocr, "{rule}");
    assert!(rule.contains("hex_app_capabilities") && rule.contains("hex_app_action script"));
    assert!(rule.contains("without its window, on any desktop"));
    assert!(rule.contains("a note by its name or text"));
    assert!(rule.contains("every note whose name contains"));
    assert!(rule.contains("never ask the person to bring it over or to retry later"));
    assert!(rule.contains("follows the autonomy rules"));
    // No app names as logic: the same rule for every app.
    for name in ["WhatsApp", "Slack", "Gmail", "Outlook", "Chrome", "Sheets"] {
        assert!(!rule.contains(name), "{name}");
        assert!(!apps_rules(true, true).contains(name), "{name}");
    }
    let mut desktop = Desktop::fixture(Some("S1"), "hex-7");
    desktop.open_apps = true;
    let asked = job(
        "Was ist das Passwort vom IP-Zugang von Kraftstoffnotdienst? Steht in Notizen",
        Origin::ActionShortcut,
        Context::default(),
    );
    assert!(system_for(&asked, Some(&desktop), true).contains(APP_METHODS));
    let mut background = asked.clone();
    background.request.origin = Origin::Background;
    desktop.open_apps = false;
    assert!(system_for(&background, Some(&desktop), true).contains(APP_METHODS));
    // A quick turn kept to the window in front hands app work to a job.
    let front_only = Desktop::fixture(Some("S1"), "hex-7");
    assert!(!system_for(&asked, Some(&front_only), true).contains(APP_METHODS));
}

#[test]
fn a_job_is_told_the_person_s_language() {
    let rule = super::language_rule(
        "Antworte Michael Schwarz auf WhatsApp und schreib ihm, dass er das rausnehmen soll",
    );
    assert!(rule.contains("speaks German"), "{rule}");
    let rule = super::language_rule("Find ten suppliers and put them in a sheet for me");
    assert!(rule.contains("speaks English"), "{rule}");
    // Too short to tell: the language of their words, never the Mac's.
    let rule = super::language_rule("Safari");
    assert!(rule.contains("in the language of their words"), "{rule}");
    assert!(!rule.contains("speaks"), "{rule}");
    // The system prompt names no language of its own.
    assert!(!system_prompt(Origin::ActionShortcut, None, false).contains("language"));
}

#[test]
fn a_backend_started_ahead_has_the_requests_own_system_prompt() {
    use crate::assistant::autonomy::{Authorized, Category};
    use crate::assistant::knowledge::{self, graph};
    let mut desktop = Desktop::fixture(Some("S1"), "hex-7");
    desktop.front_window = true;
    // As `service::prewarm` builds it: no words yet, nothing authorized.
    let mut ahead = job("", Origin::ActionShortcut, Context::default());
    ahead.settings.memory = true;
    ahead.memory = Some("- Company: blabla.com".into());
    // The request: English words on any Mac, its words allow a send, and
    // they name someone the memory graph knows.
    let mut asked = ahead.clone();
    asked.request.text = "Send the offer to Max Weber at max@example.com".into();
    asked.authorized = Authorized {
        categories: [Category::Send].into(),
        notes: vec!["only to max@example.com".into()],
        ..Authorized::default()
    };
    let mut known = graph::Graph::default();
    known.learn(
        &graph::extract("Max Weber works at blabla.com", None, "DE"),
        "remember",
        1,
    );
    asked.recall = knowledge::recall_in(&known, &asked.request.text);
    let recall = asked.recall.clone().expect("the graph knows Max Weber");
    assert_eq!(
        system_for(&ahead, Some(&desktop), true),
        system_for(&asked, Some(&desktop), true),
        "the started process must be the one the request finds"
    );
    // What only the request decides goes with its message.
    let rules = request_rules(&asked);
    assert!(rules.contains("speaks English"), "{rules}");
    assert!(
        rules.contains("For this request: Autonomy (Smart): you may only send"),
        "{rules}"
    );
    assert!(rules.contains(recall.trim()), "{rules}");
    assert!(!request_rules(&ahead).contains("For this request"));
    // (Without the desktop, so the test reads nothing of this computer.)
    asked.settings.control_desktop = false;
    let message = message(&asked, &[], false);
    assert!(message.starts_with(&rules), "{message}");
    assert!(message.ends_with("Send the offer to Max Weber at max@example.com"));
}

#[test]
fn quick_turns_run_fast_and_hand_hard_ones_to_the_strong_model() {
    let mut here = job("q", Origin::ActionShortcut, Context::default());
    here.settings.here_model = Some("claude-sonnet-5-5".into());
    here.settings.claude_code_model = Some("opus".into());
    assert_eq!(effort(&here).as_deref(), Some("low"));
    assert_eq!(escalate_after(&here), Some(8));
    let mut background = here.clone();
    background.request.origin = Origin::Background;
    assert_eq!(effort(&background), None, "a job thinks as its model does");
    assert_eq!(escalate_after(&background), None);
    // Nothing to hand over to when the fast model is the strong one.
    let mut same = here.clone();
    same.settings.here_model = Some("opus".into());
    assert_eq!(escalate_after(&same), None);
    let mut never = here.clone();
    never.settings.escalate_after_tools = 0;
    assert_eq!(escalate_after(&never), None);
    let mut api = here.clone();
    api.settings.backend = AssistantBackend::ClaudeApi;
    assert_eq!(escalate_after(&api), None);
    assert_eq!(effort(&api), None);
    // The hand-over: the same session, the strong model, a note to go on.
    let run = Run {
        model: model(&here),
        effort: effort(&here),
        escalate_after: escalate_after(&here),
        keep_warm: Some(Duration::from_secs(120)),
        image: Some("/tmp/shot.png".into()),
        ..Run::fixture(AssistantBackend::ClaudeCode)
    };
    let strong = escalated(&run, &here, Some("sess-1".into()));
    assert_eq!(strong.model.as_deref(), Some("opus"));
    assert_eq!(strong.effort, None);
    assert_eq!(strong.escalate_after, None);
    assert_eq!(strong.keep_warm, None);
    assert_eq!(strong.image, None);
    assert_eq!(strong.resume.as_deref(), Some("sess-1"));
    assert_eq!(strong.prompt, ESCALATED_NOTE);
    assert!(ESCALATED_NOTE.contains("never repeat a send"));
    // Which failures hand over.
    let handed: color_eyre::Report = provider::Escalate { tools: 8 }.into();
    assert!(should_escalate(&handed));
    assert!(should_escalate(&color_eyre::eyre::eyre!(
        "Claude Code: it stopped without an answer"
    )));
    assert!(!should_escalate(&color_eyre::eyre::eyre!("cancelled")));
    assert!(!should_escalate(&color_eyre::eyre::eyre!(
        "Claude Code: You've hit your session limit · resets 2am (Europe/Berlin)"
    )));
}

#[test]
fn the_guide_keeps_what_a_task_uses() {
    let guide = "# agent-desktop\n\nIntro.\n\n## Shell syntax\n\nPowerShell.\n\n## The loop\n\n```\n# not a heading\nagent-desktop snapshot\n```\n\n## First run\n\n1. status\n\n## Refs\n\nRefs.\n\n# agent-desktop on macOS\n\n## Requirements\n\nmacOS 13.\n\n## What differs on macOS\n\nKeys.";
    let kept = trim_guide(guide);
    for gone in ["PowerShell", "1. status", "macOS 13"] {
        assert!(!kept.contains(gone), "{gone}: {kept}");
    }
    for stays in [
        "Intro.",
        "## The loop",
        "# not a heading",
        "Refs.",
        "# agent-desktop on macOS",
        "Keys.",
    ] {
        assert!(kept.contains(stays), "{stays}: {kept}");
    }
}

/// Characters of each lane's own rules (agent-desktop's guide aside): what
/// every request pays for before its first word. Raise only with a reason.
// 9_300: the knowledge tools (hex_lookup, hex_browser_tab) add a short rule.
// 9_600: acting by a window's picture when Accessibility fails (BACKGROUND_INPUT_RULES).
// 9_700: one routing rule per lane (the web, hex_job_result) and letting go of desk windows.
const HERE_BUDGET: usize = 9_700;
// 15_900: the knowledge tools add a short rule; every app's ways, scripting first (APP_METHODS).
// 16_200: acting by a window's picture (BACKGROUND_INPUT_RULES).
// 16_500: the work-fast rules reach jobs too (speed_rules).
const JOB_BUDGET: usize = 16_500;

#[test]
fn each_lane_s_rules_stay_within_their_budget() {
    let mut desktop = Desktop::fixture(Some("S1"), "hex-7");
    desktop.front_window = true;
    desktop.open_apps = true;
    // The guide is agent-desktop's: measured apart, where it is installed.
    let without_guide = |system: String| match system.find("\nagent-desktop's guide:") {
        Some(start) => {
            let end = system[start..]
                .find("\nYou decide how")
                .or_else(|| system[start..].find("\nThis is a job"))
                .map_or(system.len(), |end| start + end);
            format!("{}{}", &system[..start], &system[end..])
        }
        None => system,
    };
    let mut here = job("q", Origin::ActionShortcut, Context::default());
    here.memory = Some("- Company: blabla.com".into());
    let quick = without_guide(system_for(&here, Some(&desktop), true));
    assert!(
        quick.len() <= HERE_BUDGET,
        "the Here lane's rules grew to {} characters",
        quick.len()
    );
    let mut background = here.clone();
    background.request.origin = Origin::Background;
    background.workspace = Some("/tmp/job".into());
    let job_rules = without_guide(system_for(&background, Some(&desktop), true));
    assert!(
        job_rules.len() <= JOB_BUDGET,
        "a job's rules grew to {} characters",
        job_rules.len()
    );
}

#[test]
#[ignore = "prints the prompt sizes per lane; run with --ignored --nocapture"]
fn print_prompt_sizes() {
    let mut desktop = Desktop::fixture(Some("S1"), "hex-7");
    desktop.front_window = true;
    desktop.open_apps = true;
    let here = job(
        "Was steht in dieser Mail?",
        Origin::ActionShortcut,
        Context::default(),
    );
    let system = system_for(&here, Some(&desktop), true);
    let guide = system
        .find("\nagent-desktop's guide:")
        .unwrap_or(system.len());
    println!("here: {} chars ({} without the guide)", system.len(), guide);
    std::fs::write(std::env::temp_dir().join("hex-here-system.txt"), &system).unwrap();
    let mut background = here.clone();
    background.request.origin = Origin::Background;
    background.workspace = Some("/tmp/job".into());
    let system = system_for(&background, Some(&desktop), true);
    let guide = system
        .find("\nagent-desktop's guide:")
        .unwrap_or(system.len());
    println!("job: {} chars ({} without the guide)", system.len(), guide);
    std::fs::write(std::env::temp_dir().join("hex-job-system.txt"), &system).unwrap();
}
