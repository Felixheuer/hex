use serde_json::json;

use super::*;

#[test]
fn secrets_are_blanked_before_a_line_is_kept() {
    let cases = [
        (
            "curl -H 'Authorization: Bearer abcdef123456' https://api.x.dev",
            "abcdef123456",
        ),
        (
            "export OPENAI_API_KEY=sk-proj-abc123def456",
            "sk-proj-abc123def456",
        ),
        (
            "git remote add o https://ghp_abcdefghij1234@github.com",
            "ghp_abcdefghij1234",
        ),
        ("psql postgres://admin:hunter2@db.local/app", "hunter2"),
        ("deploy --token=s3cr3tvalue --env prod", "s3cr3tvalue"),
        ("mysql --password hunter22 -u root", "hunter22"),
        (r#"{"api_key": "abc-123-xyz"}"#, "abc-123-xyz"),
        ("GITHUB_TOKEN=gho_1234567890 make release", "gho_1234567890"),
        (
            "https://x.dev/cb?code=1&access_token=zzz999yyy",
            "zzz999yyy",
        ),
    ];
    for (text, secret) in cases {
        let shown = redact(text);
        assert!(
            !shown.contains(secret),
            "{text:?} kept its secret: {shown:?}"
        );
        assert!(shown.contains("•••"), "{text:?} shows no blank: {shown:?}");
    }
    // Ordinary commands stay as they were.
    for plain in [
        "cargo test --bin agent-desk view::",
        "ls -la src",
        "git status",
    ] {
        assert_eq!(redact(plain), plain);
    }
    let line = TermLine::new(Ink::Prompt, "TOKEN=abcdef1234 ./run.sh");
    assert!(!line.text.contains("abcdef1234"));
}

#[test]
fn the_terminal_keeps_its_last_lines_only() {
    let mut console = Console::default();
    let lines: Vec<TermLine> = (0..CONSOLE_MAX + 250)
        .map(|n| TermLine::new(Ink::Output, &format!("line {n}")))
        .collect();
    for chunk in lines.chunks(100) {
        console.push(chunk);
    }
    assert_eq!(console.len(), CONSOLE_MAX);
    assert_eq!(console.lines().next().unwrap().text, "line 250");
    let tail = console.tail(2);
    assert_eq!(tail[1].text, format!("line {}", CONSOLE_MAX + 249));
}

#[test]
fn calls_make_terminal_lines_by_their_kind() {
    let run = call_lines(
        ToolKind::Command,
        &json!({"command": "cargo test view::\ncat <<EOF\nbody\nEOF", "description": "Runs tests"}),
    );
    assert_eq!(run, vec![TermLine::new(Ink::Prompt, "cargo test view::")]);
    let described = call_lines(ToolKind::Command, &json!({"description": "Lists files"}));
    assert_eq!(described[0].text, "Lists files");
    let search = call_lines(ToolKind::Search, &json!({"pattern": "Kontakte verknüpfen"}));
    assert_eq!(
        search,
        vec![TermLine::new(Ink::Searched, "Kontakte verknüpfen")]
    );
    let read = call_lines(
        ToolKind::Read,
        &json!({"file_path": "/repo/src/setup_gaps.rs"}),
    );
    assert_eq!(read, vec![TermLine::new(Ink::Read, "setup_gaps.rs")]);
    let edit = call_lines(
        ToolKind::Edit,
        &json!({"file_path": "src/x.rs", "old_string": "let a = 1;\nlet b = 2;", "new_string": "let a = 3;"}),
    );
    assert_eq!(
        edit[0],
        TermLine::new(
            Ink::Edited {
                added: Some(1),
                removed: Some(2)
            },
            "x.rs"
        )
    );
    assert!(edit.len() <= 1 + DIFF_LINES);
    assert!(edit.iter().any(|line| line.ink == Ink::Removed));
    assert!(
        edit.iter()
            .any(|line| line.ink == Ink::Added && line.text == "let a = 3;")
    );
    // A message typed or a click makes no line: no body is ever logged.
    assert!(call_lines(ToolKind::Desktop, &json!({"text": "Dear Anna, …"})).is_empty());
    assert!(call_lines(ToolKind::Other, &json!({"body": "hello"})).is_empty());
}

#[test]
fn a_commands_output_keeps_its_tail_and_others_none() {
    let output = "one\n\x1b[32mtwo\x1b[0m\nthree\n\nfour\nfive\nAPI_KEY=abc123456\n";
    let lines = output_lines(ToolKind::Command, output);
    assert_eq!(lines.len(), OUTPUT_TAIL);
    assert_eq!(lines[0].text, "three");
    assert!(lines.iter().all(|line| line.ink == Ink::Output));
    assert!(!lines[3].text.contains("abc123456"));
    assert!(output_lines(ToolKind::Read, output).is_empty());
}
