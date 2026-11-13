//! The desktop's self-tests: Cua Driver is installed and acts with
//! Agent-Desk's grants, a round trip that lists apps and windows through it,
//! and the same desktop tool called through an agent (the guard's,
//! read only).

use super::*;

/// Cua Driver, or the check failing with its install fix.
fn driver(check: Check) -> Result<PathBuf, CheckResult> {
    if !cfg!(target_os = "macos") {
        return Err(CheckResult::failed(
            check,
            "Cua Driver, the desktop driver, runs on macOS only",
            None,
        ));
    }
    crate::assistant::driver::find().ok_or_else(|| {
        CheckResult::failed(
            check,
            "Cua Driver, the desktop driver, isn't installed",
            Some(Fix::InstallDriver),
        )
    })
}

/// What a permissions reply means for a check.
pub(super) fn judge_grants(check: Check, grants: desktop::Grants, who: &str) -> CheckResult {
    match (grants.accessibility, grants.screen_recording) {
        (true, true) => CheckResult::passed(
            check,
            format!("Accessibility and Screen Recording are granted ({who})"),
        ),
        (false, _) => CheckResult::failed(
            check,
            format!(
                "Cua Driver has no Accessibility ({who}). Switch Agent-Desk on in System Settings"
            ),
            Some(Fix::OpenSettings(Permission::Accessibility)),
        ),
        (true, false) => CheckResult::failed(
            check,
            format!("Cua Driver has no Screen Recording ({who}), so agents can't take screenshots"),
            Some(Fix::OpenSettings(Permission::ScreenRecording)),
        ),
    }
}

pub(super) fn desktop_permissions(check: Check) -> CheckResult {
    if let Err(failed) = driver(check) {
        return failed;
    }
    // Cua Driver runs inside Agent-Desk's process tree: Agent-Desk's grants are its.
    let permissions = computer::permissions_now();
    let mut result = judge_grants(
        check,
        desktop::Grants {
            accessibility: permissions.accessibility,
            screen_recording: permissions.screen_recording,
        },
        "acting with Agent-Desk's grants",
    );
    if result.ok() {
        result.detail = format!(
            "{}; {}",
            result.detail,
            crate::assistant::driver::update::status().line()
        );
    }
    result
}

pub(super) fn round_trip(check: Check) -> CheckResult {
    if let Err(failed) = driver(check) {
        return failed;
    }
    match crate::assistant::guard::driver_round_trip() {
        Ok((apps, windows)) if apps > 0 => CheckResult::passed(
            check,
            format!("Cua Driver: {apps} apps running, {windows} windows"),
        ),
        Ok(_) => failed_round_trip(check, "list the running apps", "it listed none"),
        Err(error) => failed_round_trip(check, "list the apps and windows", &error),
    }
}

pub(super) fn failed_round_trip(check: Check, what: &str, why: &str) -> CheckResult {
    let fix = if why.contains("PERM") || why.to_ascii_lowercase().contains("permission") {
        Fix::OpenSettings(Permission::Accessibility)
    } else {
        Fix::Retry
    };
    CheckResult::failed(
        check,
        format!("Cua Driver couldn't {what}: {why}"),
        Some(fix),
    )
}

/// The prompt that has an agent call `desktop_status` under its own name
/// for the tool.
pub(super) fn status_prompt(tool: &str) -> String {
    format!(
        "Call the {tool} tool once, with no arguments. Then reply with only the value of \
         permissions.accessibility.state from its result. Do nothing else."
    )
}

pub(super) fn through_agent(
    check: Check,
    backend: AssistantBackend,
    settings: &AssistantSettings,
) -> CheckResult {
    let path = match installed(check, backend) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    if let Err(failed) = driver(check) {
        return failed;
    }
    // The agent calls the guard's desktop tools (read only), which serve
    // `desktop_status` from Cua Driver.
    let Some(server) = crate::assistant::guard::check_server() else {
        return CheckResult::failed(
            check,
            "Cua Driver, the desktop driver, isn't installed",
            Some(Fix::InstallDriver),
        );
    };
    let mut command = agent_command(&path);
    command.current_dir(scratch());
    match backend {
        AssistantBackend::ClaudeCode => {
            let tool = format!("mcp__{}__desktop_status", super::super::MCP_NAME);
            command.args([
                "-p",
                &status_prompt(&tool),
                "--output-format",
                "stream-json",
            ]);
            command.args(["--verbose", "--tools", "", "--no-session-persistence"]);
            command.args([
                "--mcp-config",
                &server.claude_config(),
                "--strict-mcp-config",
            ]);
            command.args(["--allowedTools", &tool]);
            if let Some(model) = &settings.claude_code_model {
                command.args(["--model", model]);
            }
        }
        AssistantBackend::Codex => {
            command.args(codex_base(&scratch()));
            command.args(server.codex_args(&["desktop_status"]));
            if let Some(model) = &settings.codex_model {
                command.args(["--model", model]);
            }
            command.arg(status_prompt("desktop_status"));
        }
        AssistantBackend::OpenCode => {
            let tool = format!("{}_desktop_status", super::super::MCP_NAME);
            let config = serde_json::json!({
                "mcp": server.opencode_config(),
                "tools": { "*": false, tool.as_str(): true },
            });
            let model = super::super::providers::opencode_check(&mut command, settings, config);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(status_prompt(&tool));
        }
        AssistantBackend::ClaudeApi => {
            return CheckResult::passed(
                check,
                "Agent-Desk runs the desktop tools for the Claude API itself",
            );
        }
    }
    let ran = match run::bounded(command, MCP_LIMIT) {
        Ok(ran) => ran,
        Err(error) => {
            return CheckResult::failed(
                check,
                format!("{} didn't start: {error}", name(backend)),
                Some(Fix::Retry),
            );
        }
    };
    if let Some(status) = desktop::status_in_events(&ran.stdout) {
        return match desktop::grants(&status) {
            Some(grants) => {
                let who = format!("started by {} from Agent-Desk", name(backend));
                let mut result = judge_grants(check, grants, &who);
                if result.ok() {
                    result.detail = format!(
                        "Called the desktop tools in {:.1} s; Cua Driver has Accessibility and Screen Recording",
                        ran.took.as_secs_f32()
                    );
                }
                result
            }
            None => CheckResult::failed(
                check,
                "The desktop tools answered without their permissions",
                Some(Fix::Retry),
            ),
        };
    }
    let events = parse_events(&ran.stdout);
    let problem = mcp_error(&events)
        .or_else(|| event_error(&events))
        .unwrap_or_else(|| {
            if ran.timed_out() {
                format!("No reply within {} s", MCP_LIMIT.as_secs())
            } else {
                let said = answer_text(&events);
                if said.trim().is_empty() {
                    format!("It never called the desktop tools. {}", ran.last_words())
                } else {
                    format!(
                        "It never called the desktop tools; it said “{}”",
                        run::shorten(said.trim(), 80)
                    )
                }
            }
        });
    let fix = if signed_out(&problem) {
        sign_in_fix(backend)
    } else {
        Fix::Retry
    };
    CheckResult::failed(check, problem, Some(fix))
}

/// A failed MCP tool call in Codex's events ("requires approval", a server
/// that didn't start).
pub(super) fn mcp_error(events: &[Value]) -> Option<String> {
    events.iter().find_map(|event| {
        let item = event.get("item")?;
        (item.get("type").and_then(Value::as_str) == Some("mcp_tool_call"))
            .then(|| item["error"].get("message").and_then(Value::as_str))
            .flatten()
            .map(|message| format!("The desktop tool call failed: {message}"))
    })
}
