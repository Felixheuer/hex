//! Tests of the task files, the checks, the results, and the CLI.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;

use super::check::{self, Observation, ToolCall, Win};
use super::results::{self, RunResult, Status, TaskResult};
use super::run;
use super::task::{self, Check, Config, Lang, Task};
use super::*;

fn repo_bench() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bench")
}

fn example_config() -> Config {
    task::load_config(&repo_bench().join("config.example.toml"))
        .unwrap()
        .unwrap()
}

pub(super) fn parse(text: &str) -> Task {
    task::parse(text).unwrap()
}

pub(super) const QUICK: &str = r#"
id = "quick"
title = "Quick"
lane = "quick"
timeout_s = 30
[text]
de = "Wie viel ist 17 mal 23?"
en = "What is 17 times 23?"
[[check]]
kind = "answered"
[[check]]
kind = "answer_matches"
pattern = "391"
"#;

// ---------------------------------------------------------------- tasks

#[test]
fn the_repository_tasks_load_and_keep_the_rules() {
    let tasks = task::load_dir(&repo_bench().join("tasks")).unwrap();
    assert_eq!(tasks.len(), 25, "about 25 tasks");
    let problems = task::validate(&tasks, Some(&example_config()));
    assert!(problems.is_empty(), "{problems:#?}");
    // Every task's needs are in the example config, so a filled-in config
    // runs them all.
    for task in &tasks {
        assert!(task.missing(&example_config()).is_empty(), "{}", task.id);
    }
}

#[test]
fn the_repository_tasks_cover_the_roadmap() {
    let tasks = task::load_dir(&repo_bench().join("tasks")).unwrap();
    let ids: Vec<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
    for wanted in [
        "open-app-parked",
        "switch-to-app",
        "park-app",
        "other-space",
        "whatsapp-draft",
        "web-three-browsers",
        "supplier-csv",
        "supplier-numbers",
        "note",
        "find-file-reveal",
        "calendar-read",
        "chrome-tab",
        "ocr-window",
        "answer-during-job",
        "follow-up",
        "language-explain",
        "quick-math",
    ] {
        assert!(ids.contains(&wanted), "{wanted} is missing");
    }
    // Message tasks are drafts with a check against sending.
    for task in tasks
        .iter()
        .filter(|task| task.safety.messages == task::Messages::Draft)
    {
        assert!(
            task.checks
                .iter()
                .any(|check| matches!(check, Check::ToolNotUsed { .. })),
            "{}",
            task.id
        );
    }
}

#[test]
fn a_dry_run_of_the_repository_tasks_passes() {
    let tasks = task::load_dir(&repo_bench().join("tasks")).unwrap();
    let config = example_config();
    let problems = task::validate(&tasks, Some(&config));
    let args = RunArgs {
        filter: None,
        target: Target::Dev,
        lang: LangArg::Both,
        dry_run: true,
        allow_front: false,
        dir: Some(repo_bench()),
        json: false,
    };
    assert_eq!(dry_run(&tasks, &config, "example", &problems, &args), 0);
}

fn problems_of(text: &str) -> Vec<String> {
    task::validate(&[parse(text)], None)
        .into_iter()
        .map(|problem| problem.message)
        .collect()
}

#[test]
fn validation_refuses_literal_recipients_and_outside_paths() {
    let phone = QUICK.replace(
        "Wie viel ist 17 mal 23?",
        "Schreib +49 170 1234567 eine Nachricht",
    );
    assert!(
        problems_of(&phone)
            .iter()
            .any(|p| p.contains("phone number"))
    );
    let email = QUICK.replace("What is 17 times 23?", "Mail anna@example.com now");
    assert!(problems_of(&email).iter().any(|p| p.contains("address")));
    let path = QUICK.replace("What is 17 times 23?", "Save it to ~/Desktop/list.csv");
    assert!(problems_of(&path).iter().any(|p| p.contains("outside")));
    assert!(problems_of(QUICK).is_empty());
}

#[test]
fn validation_requires_drafts_to_guard_against_sending() {
    let draft = r#"
id = "draft"
title = "Draft"
timeout_s = 60
[safety]
messages = "draft"
[text]
de = "Schreib {test_contact} einen Entwurf"
en = "Draft a message to {test_contact}"
[[check]]
kind = "answered"
"#;
    assert!(
        problems_of(draft)
            .iter()
            .any(|p| p.contains("tool_not_used"))
    );
    let guarded = format!("{draft}[[check]]\nkind = \"tool_not_used\"\npattern = \"send\"\n");
    assert!(
        problems_of(&guarded).is_empty(),
        "{:?}",
        problems_of(&guarded)
    );
    // Naming a recipient while claiming no messages.
    let sneaky = guarded.replace("messages = \"draft\"", "messages = \"none\"");
    assert!(problems_of(&sneaky).iter().any(|p| p.contains("recipient")));
    // A draft that names no test recipient.
    let anyone = guarded.replace("{test_contact}", "Anna");
    assert!(
        problems_of(&anyone)
            .iter()
            .any(|p| p.contains("{test_contact}"))
    );
}

#[test]
fn validation_checks_patterns_order_and_preconditions() {
    let bad_regex = QUICK.replace("pattern = \"391\"", "pattern = \"(391\"");
    assert!(
        problems_of(&bad_regex)
            .iter()
            .any(|p| p.contains("compile"))
    );
    let timeout = QUICK.replace("timeout_s = 30", "timeout_s = 99999");
    assert!(
        problems_of(&timeout)
            .iter()
            .any(|p| p.contains("timeout_s"))
    );
    let precondition = format!("{QUICK}[[precondition]]\nkind = \"answered\"\n");
    assert!(
        problems_of(&precondition)
            .iter()
            .any(|p| p.contains("precondition"))
    );
    let follow = QUICK.replace("lane = \"quick\"", "lane = \"quick\"\nafter = \"later\"");
    assert!(
        problems_of(&follow)
            .iter()
            .any(|p| p.contains("earlier task"))
    );
    let file = format!("{QUICK}[[check]]\nkind = \"file\"\nglob = \"*.csv\"\n");
    assert!(problems_of(&file).iter().any(|p| p.contains("{temp_dir}")));
    let duplicate = task::validate(&[parse(QUICK), parse(QUICK)], None);
    assert!(duplicate.iter().any(|p| p.message.contains("taken")));
    // Unknown fields are refused, so a typo can't silently drop a rule.
    assert!(task::parse(&QUICK.replace("lane =", "lanes =")).is_err());
}

#[test]
fn placeholders_fill_escaped_except_expressions() {
    let values: BTreeMap<String, String> = [
        ("name".to_owned(), "a.b".to_owned()),
        ("when_re".to_owned(), "\\d+".to_owned()),
    ]
    .into();
    assert_eq!(
        task::fill("x {name} {when_re}", &values, true).unwrap(),
        "x a\\.b \\d+"
    );
    assert_eq!(task::fill("x {name}", &values, false).unwrap(), "x a.b");
    assert_eq!(
        task::fill("{missing}", &values, false),
        Err("missing".to_owned())
    );
    let config = task::parse_config("a = \"1\"\nempty = \"\"\nn = 3\n").unwrap();
    assert_eq!(config.get("n").map(String::as_str), Some("3"));
    assert!(!config.contains_key("empty"));
}

// ---------------------------------------------------------------- checks

fn values() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[test]
fn answer_checks_read_the_answer() {
    let seen = Observation {
        lang: Some(Lang::De),
        outcome: "answered".into(),
        answer: "Das ist 391, und es ist nicht schwer.".into(),
        first_answer_ms: Some(1200),
        ..Observation::default()
    };
    let passes = |check: Check| check::evaluate(&check, &seen, &values()).passed;
    assert!(passes(Check::Answered));
    assert!(passes(Check::AnswerMatches {
        pattern: "391".into(),
        lang: None
    }));
    // A check for the other language doesn't apply.
    assert!(passes(Check::AnswerMatches {
        pattern: "never".into(),
        lang: Some(Lang::En)
    }));
    assert!(passes(Check::AnswerLanguage));
    assert!(passes(Check::AnswerWithinMs { ms: 1500 }));
    assert!(!passes(Check::AnswerWithinMs { ms: 1000 }));
}

#[test]
fn language_is_told_by_common_words() {
    assert_eq!(
        check::language("Der Kondensator speichert elektrische Ladung."),
        Some(Lang::De)
    );
    assert_eq!(
        check::language("A capacitor stores electric charge in a field."),
        Some(Lang::En)
    );
    assert_eq!(check::language("391"), None);
}

#[test]
fn globs_match_names() {
    assert!(check::glob_match("*.csv", "Lieferanten.CSV"));
    assert!(check::glob_match("bench-?.txt", "bench-1.txt"));
    assert!(!check::glob_match("*.csv", "list.numbers"));
    assert!(check::glob_match("*", "anything"));
}

#[test]
fn file_checks_look_in_the_temp_folder() {
    let dir = std::env::temp_dir().join(format!("hex-bench-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/list.csv"), "name,url\nA,https://a.de\n").unwrap();
    let seen = Observation {
        temp_dir: dir.clone(),
        ..Observation::default()
    };
    let file = |contains: Option<&str>, min| Check::File {
        glob: "*.csv".into(),
        contains: contains.map(str::to_owned),
        min,
        root: task::FileRoot::Temp,
    };
    assert!(check::evaluate(&file(Some("https://"), 1), &seen, &values()).passed);
    assert!(!check::evaluate(&file(Some("nowhere"), 1), &seen, &values()).passed);
    assert!(!check::evaluate(&file(None, 2), &seen, &values()).passed);
    let workspace = Check::File {
        glob: "*".into(),
        contains: None,
        min: 1,
        root: task::FileRoot::Workspace,
    };
    assert!(!check::evaluate(&workspace, &seen, &values()).passed);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tool_and_url_checks_read_the_audit() {
    let call = |tool: &str, input: &str| ToolCall {
        tool: tool.into(),
        input: input.into(),
    };
    let seen = Observation {
        tools: vec![
            call(
                "mcp__browser__navigate",
                r#"{"url":"https://duckduckgo.com/?q=rust"}"#,
            ),
            call(
                "mcp__browser__navigate",
                r#"{"url":"https://www.google.com/search?q=rust"}"#,
            ),
            call(
                "mcp__browser__navigate",
                r#"{"url":"https://duckduckgo.com/?q=rust"}"#,
            ),
            call("mcp__hex-desktop__desktop_type", r#"{"text":"Bench"}"#),
        ],
        actions: vec!["Opened https://www.bing.com/search?q=rust".into()],
        ..Observation::default()
    };
    let passes = |check: Check| check::evaluate(&check, &seen, &values()).passed;
    assert!(passes(Check::BrowserUrl {
        pattern: "^https://".into(),
        min: 3
    }));
    assert!(!passes(Check::BrowserUrl {
        pattern: "^https://".into(),
        min: 4
    }));
    assert!(passes(Check::ToolUsed {
        pattern: "desktop_type".into()
    }));
    let no_send =
        r"(?i)((press|key)[^\n]{0,80}\b(return|enter)\b|click[^\n]{0,160}\b(senden|send)\b)";
    assert!(passes(Check::ToolNotUsed {
        pattern: no_send.into()
    }));
    let mut sent = seen.clone();
    sent.tools.push(call(
        "mcp__hex-desktop__desktop_press_key",
        r#"{"key":"Return"}"#,
    ));
    assert!(
        !check::evaluate(
            &Check::ToolNotUsed {
                pattern: no_send.into()
            },
            &sent,
            &values()
        )
        .passed
    );
    assert_eq!(seen.steps(), 4);
}

#[test]
fn desktop_checks_read_apps_windows_and_the_front() {
    let seen = Observation {
        front_before: Some("Google Chrome".into()),
        fronts: vec!["Google Chrome".into()],
        front_after: Some("Google Chrome".into()),
        apps_after: Some(vec!["Finder".into(), "Text\u{200e}Edit".into()]),
        windows_after: [(
            "Finder".to_owned(),
            vec![Win {
                app: "Finder".into(),
                title: "find-file-reveal-de".into(),
                visible: true,
            }],
        )]
        .into(),
        ..Observation::default()
    };
    let passes = |check: Check| check::evaluate(&check, &seen, &values()).passed;
    assert!(passes(Check::AppRunning {
        app: "TextEdit".into()
    }));
    assert!(!passes(Check::AppRunning {
        app: "Numbers".into()
    }));
    let window = |title: &str, present| Check::Window {
        app: "Finder".into(),
        title: Some(title.into()),
        visible: None,
        present,
    };
    assert!(passes(window("find-file-reveal", true)));
    assert!(passes(window("elsewhere", false)));
    assert!(passes(Check::FrontApp {
        app: "TextEdit".into(),
        is: false
    }));
    assert!(check::front_kept(&seen).passed);
    let mut moved = seen.clone();
    moved.fronts.push("TextEdit".into());
    let verdict = check::front_kept(&moved);
    assert!(!verdict.passed);
    assert!(verdict.detail.contains("TextEdit"));
}

#[test]
fn plans_are_read_from_markdown() {
    let plan = run::parse_plan("# Plan\n- [x] Search\n- [ ] Write\n* [X] Check\nnotes\n- [ ]\n");
    assert_eq!(
        plan,
        vec![
            (true, "Search".to_owned()),
            (false, "Write".to_owned()),
            (true, "Check".to_owned())
        ]
    );
    let seen = Observation {
        plan,
        ..Observation::default()
    };
    let check = |all_done| Check::Plan {
        min_steps: 2,
        all_done,
    };
    assert!(check::evaluate(&check(false), &seen, &values()).passed);
    assert!(!check::evaluate(&check(true), &seen, &values()).passed);
}

// ---------------------------------------------------------------- results

fn result(id: &str, status: Status, first: Option<u64>) -> TaskResult {
    TaskResult {
        first_answer_ms: first,
        total_ms: first.map(|first| first * 2),
        steps: Some(1),
        ..TaskResult::new(id, Lang::De, status, "")
    }
}

fn run_of(started_ms: u64, tasks: Vec<TaskResult>) -> RunResult {
    RunResult {
        schema: results::SCHEMA,
        version: "0.9.1".into(),
        target: "dev".into(),
        run_id: timestamp(started_ms),
        started_ms,
        finished_ms: started_ms + 1,
        filter: None,
        tasks,
    }
}

#[test]
fn a_task_that_stops_passing_makes_the_run_worse() {
    let before = run_of(
        1,
        vec![
            result("a", Status::Passed, Some(1000)),
            result("b", Status::Failed, None),
        ],
    );
    let now = run_of(
        2,
        vec![
            result("a", Status::Failed, Some(900)),
            result("b", Status::Passed, Some(800)),
        ],
    );
    let comparison = results::compare(&now, Some(&before));
    assert!(comparison.rows[0].regressed());
    assert!(comparison.rows[1].fixed());
    assert!(comparison.worse());
    let table = results::table(&now, Some(&before), &comparison);
    assert!(table.contains("▼") && table.contains("▲"), "{table}");
    assert!(table.contains("Worse than before."));
    // Better or equal is not worse.
    let better = run_of(
        3,
        vec![
            result("a", Status::Passed, Some(900)),
            result("b", Status::Passed, Some(800)),
        ],
    );
    assert!(!results::compare(&better, Some(&before)).worse());
    assert!(!results::compare(&better, None).worse());
}

#[test]
fn a_filtered_or_skipping_run_is_not_worse_for_what_it_left_out() {
    let before = run_of(
        1,
        vec![
            result("a", Status::Passed, Some(1000)),
            result("b", Status::Passed, Some(1000)),
        ],
    );
    let filtered = run_of(2, vec![result("a", Status::Passed, Some(1000))]);
    assert!(!results::compare(&filtered, Some(&before)).worse());
    let skipped = run_of(
        2,
        vec![
            result("a", Status::Passed, Some(1000)),
            result("b", Status::Skipped, None),
        ],
    );
    let comparison = results::compare(&skipped, Some(&before));
    assert!(!comparison.rows[1].regressed());
}

#[test]
fn results_are_saved_and_found_in_order() {
    let dir = std::env::temp_dir().join(format!("hex-bench-results-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let older = run_of(
        1_700_000_000_000,
        vec![result("a", Status::Passed, Some(5))],
    );
    let newer = run_of(1_700_000_100_000, vec![result("a", Status::Failed, None)]);
    let path = results::save(&dir, &newer).unwrap();
    results::save(&dir, &older).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "0.9.1-20231114-221500.json"
    );
    let all = results::load_all(&dir);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].1, older);
    assert_eq!(all[1].1, newer);
    assert_eq!(previous(&dir, &newer), Some(older));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn timestamps_are_utc_calendar_times() {
    assert_eq!(timestamp(0), "19700101-000000");
    assert_eq!(timestamp(1_700_000_000_000), "20231114-221320");
    assert_eq!(timestamp(951_782_400_000), "20000229-000000");
}

// ---------------------------------------------------------------- CLI

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: BenchCommand,
}

#[test]
fn the_cli_defaults_to_the_dev_app() {
    let Cli {
        command: BenchCommand::Run(args),
    } = Cli::try_parse_from(["bench", "run", "--dry-run"]).unwrap()
    else {
        panic!("not run");
    };
    assert_eq!(args.target, Target::Dev);
    assert!(args.dry_run);
    assert_eq!(args.lang, LangArg::De);
    let Cli {
        command: BenchCommand::Run(args),
    } = Cli::try_parse_from([
        "bench", "run", "--target", "live", "--filter", "quick", "--lang", "both",
    ])
    .unwrap()
    else {
        panic!("not run");
    };
    assert_eq!(args.target, Target::Live);
    assert_eq!(args.filter.as_deref(), Some("quick"));
    assert!(Cli::try_parse_from(["bench", "run", "--target", "prod"]).is_err());
    assert!(matches!(
        Cli::try_parse_from(["bench", "report", "--fail-on-regression"])
            .unwrap()
            .command,
        BenchCommand::Report {
            fail_on_regression: true,
            ..
        }
    ));
}

#[test]
fn a_filter_takes_the_tasks_followed_up_on() {
    let tasks = task::load_dir(&repo_bench().join("tasks")).unwrap();
    let ids = |filter| -> Vec<String> {
        select(tasks.clone(), Some(filter))
            .unwrap()
            .into_iter()
            .map(|task| task.id)
            .collect()
    };
    assert_eq!(ids("^follow-up$"), vec!["quick-math", "follow-up"]);
    assert_eq!(ids("^park-app$"), vec!["switch-to-app", "park-app"]);
    assert!(select(tasks, Some("(")).is_err());
}

// ---------------------------------------------------------------- live parsing

#[test]
fn the_drivers_listings_and_lsappinfo_replies_are_read() {
    let apps = serde_json::json!({"apps": [{"name": "Finder"}, {"name": "Citrix Viewer"}]});
    assert_eq!(live::apps_of(&apps), vec!["Finder", "Citrix Viewer"]);
    let windows: serde_json::Value = serde_json::from_str(
        r#"[{"id":"w-1","app_name":"Finder","title":"bench","visible":false}]"#,
    )
    .unwrap();
    assert_eq!(
        live::windows_of(&windows),
        vec![Win {
            app: "Finder".into(),
            title: "bench".into(),
            visible: false
        }]
    );
    assert_eq!(
        live::display_name("\"LSDisplayName\"=\"Google Chrome\"\n").as_deref(),
        Some("Google Chrome")
    );
    assert_eq!(live::display_name(""), None);
}

#[test]
fn the_audit_log_is_read_without_a_jobs_own_checks() {
    let dir = std::env::temp_dir().join(format!("hex-bench-audit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.jsonl");
    std::fs::write(
        &path,
        "{\"at_ms\":1,\"tool\":\"Bash\",\"input\":\"ls\"}\n{\"at_ms\":2,\"tool\":\"desktop_snapshot\",\"input\":\"{}\",\"check\":true}\nnot json\n",
    )
    .unwrap();
    assert_eq!(
        live::read_audit(&path),
        vec![ToolCall {
            tool: "Bash".into(),
            input: "ls".into()
        }]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_dev_target_is_agent_desk_devs_data_folder() {
    if std::env::var_os("AGENT_DESK_BENCH_SUPPORT_DIR").is_some() {
        return;
    }
    let dev = Target::Dev.support_dir().unwrap();
    let live = Target::Live.support_dir().unwrap();
    assert!(dev.ends_with("agent-desk-dev"), "{}", dev.display());
    assert!(live.ends_with("agent-desk"), "{}", live.display());
}
