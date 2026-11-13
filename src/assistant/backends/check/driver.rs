//! The desktop's self-tests: agent-desktop's (or Cua Driver's) permissions,
//! a round trip that lists apps and windows and reads the Dock, and the
//! same desktop tool called through an agent.

use super::*;

/// agent-desktop, or the check failing with its install fix.
pub(super) fn agent_desktop(check: Check) -> Result<PathBuf, CheckResult> {
    search::find("agent-desktop").ok_or_else(|| {
        CheckResult::failed(
            check,
            "agent-desktop isn't installed",
            Some(Fix::InstallAgentDesktop),
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
        (false, _) if cfg!(target_os = "linux") => CheckResult::failed(
            check,
            format!(
                "agent-desktop can't read apps ({who}): turn on assistive technologies \
                 (AT-SPI), e.g. gsettings set org.gnome.desktop.interface \
                 toolkit-accessibility true, then sign out and in"
            ),
            None,
        ),
        (false, _) => CheckResult::failed(
            check,
            format!(
                "agent-desktop has no Accessibility ({who}). Switch Agent-Desk on in System Settings"
            ),
            Some(Fix::GuidePermissions),
        ),
        (true, false) => CheckResult::failed(
            check,
            format!(
                "agent-desktop has no Screen Recording ({who}), so agents can't take screenshots"
            ),
            Some(Fix::GuidePermissions),
        ),
    }
}

pub(super) fn desktop_permissions(check: Check) -> CheckResult {
    // Cua Driver runs inside Agent-Desk's process tree: Agent-Desk's grants are its.
    if cua_active() {
        let permissions = computer::permissions_now();
        return judge_grants(
            check,
            desktop::Grants {
                accessibility: permissions.accessibility,
                screen_recording: permissions.screen_recording,
            },
            "Cua Driver, acting with Agent-Desk's grants",
        );
    }
    let path = match agent_desktop(check) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let grants = super::super::agent_desktop_grants(&path);
    // On Linux agent-desktop reads apps through the AT-SPI accessibility
    // bus; when it can't, a missing bus is the likely reason.
    #[cfg(target_os = "linux")]
    if !grants.as_ref().is_ok_and(|grants| grants.accessibility)
        && let Some(Err(why)) = atspi_bus()
    {
        return CheckResult::failed(check, why, Some(Fix::Retry));
    }
    match grants {
        Ok(grants) => judge_grants(check, grants, "started by Agent-Desk"),
        Err(error) => CheckResult::failed(
            check,
            format!("agent-desktop didn't answer: {error}"),
            Some(Fix::InstallAgentDesktop),
        ),
    }
}

/// Whether the session's AT-SPI accessibility bus answers (`org.a11y.Bus`
/// on the session D-Bus); `None` when there's nothing to ask it with.
#[cfg(target_os = "linux")]
pub(super) fn atspi_bus() -> Option<Result<(), String>> {
    const MISSING: &str = "The accessibility bus (AT-SPI) isn't running, so agent-desktop \
                           can't read apps. Install at-spi2-core and sign in to a desktop \
                           session again";
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none()
        && std::env::var_os("XDG_RUNTIME_DIR")
            .is_none_or(|dir| !Path::new(&dir).join("bus").exists())
    {
        return Some(Err(
            "There's no desktop session bus (D-Bus), so agent-desktop can't read apps".into(),
        ));
    }
    let gdbus: &[&str] = &[
        "call",
        "--session",
        "--dest",
        "org.a11y.Bus",
        "--object-path",
        "/org/a11y/bus",
        "--method",
        "org.a11y.Bus.GetAddress",
    ];
    let dbus_send: &[&str] = &[
        "--session",
        "--print-reply",
        "--dest=org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Bus.GetAddress",
    ];
    let (program, args) = search::find("gdbus")
        .map(|program| (program, gdbus))
        .or_else(|| search::find("dbus-send").map(|program| (program, dbus_send)))?;
    let mut command = crate::desktop::background_process::command(program);
    command.args(args);
    let ran = run::bounded(command, Duration::from_secs(5)).ok()?;
    Some(if ran.success() {
        Ok(())
    } else {
        Err(MISSING.into())
    })
}

/// Whether Cua Driver is the desktop driver that runs.
pub(super) fn cua_active() -> bool {
    crate::assistant::driver::active()
        .is_some_and(|found| found.driver == crate::assistant::driver::Driver::Cua)
}

pub(super) fn round_trip(check: Check) -> CheckResult {
    if cua_active() {
        return match crate::assistant::guard::driver_round_trip() {
            Ok((apps, windows)) if apps > 0 => CheckResult::passed(
                check,
                format!("Cua Driver: {apps} apps running, {windows} windows"),
            ),
            Ok(_) => failed_round_trip(check, "list the running apps", "it listed none"),
            Err(error) => failed_round_trip(check, "list the apps and windows", &error),
        };
    }
    let path = match agent_desktop(check) {
        Ok(path) => path,
        Err(failed) => return failed,
    };
    let call = |args: &[&str]| -> Result<Value, String> {
        match super::super::desktop_call(&path, args)? {
            desktop::Reply::Ok(data) => Ok(data),
            desktop::Reply::Failed { code, message } => Err(format!("{code}: {message}")),
        }
    };
    let (apps, foreground) = match call(&["list-apps"]) {
        Ok(data) => (desktop::apps(&data), desktop::foreground_apps(&data)),
        Err(error) => return failed_round_trip(check, "list the running apps", &error),
    };
    if apps.is_empty() {
        return failed_round_trip(check, "list the running apps", "it listed none");
    }
    // Only macOS has a Dock.
    let dock = if cfg!(target_os = "macos") {
        let dock = match call(&["snapshot", "--surface", "dock", "--skeleton", "--compact"]) {
            Ok(data) => data.get("ref_count").and_then(Value::as_u64).unwrap_or(0),
            Err(error) => return failed_round_trip(check, "read the Dock", &error),
        };
        if dock == 0 {
            return failed_round_trip(check, "read the Dock", "it found nothing in it");
        }
        Some(dock)
    } else {
        None
    };
    let windows = match call(&["list-windows"]) {
        Ok(data) => desktop::windows(&data),
        // One busy background app (Universal Control, a menu bar tool whose
        // windows come and go) keeps the global inventory from settling,
        // though every app a task works in lists fine on its own.
        Err(error) if error.starts_with("TIMEOUT") => {
            match windows_app_by_app(&call, &foreground) {
                Some(windows) => windows,
                None => return failed_round_trip(check, "list the windows", &error),
            }
        }
        Err(error) => return failed_round_trip(check, "list the windows", &error),
    };
    let visible: Vec<_> = windows.iter().filter(|window| window.visible).collect();
    let outlined = visible.iter().find(|window| {
        computer::window_frame(&crate::assistant::TaskWindow {
            app: window.app.clone(),
            window_id: Some(window.id.clone()),
        })
        .is_some()
    });
    if !visible.is_empty() && outlined.is_none() {
        return CheckResult::failed(
            check,
            "Agent-Desk can't find agent-desktop's windows on screen, so task outlines won't show",
            Some(Fix::Retry),
        );
    }
    let dock = dock
        .map(|dock| format!(", {dock} items in the Dock"))
        .unwrap_or_default();
    CheckResult::passed(
        check,
        format!(
            "{} apps running, {} windows{dock}",
            apps.len(),
            windows.len()
        ),
    )
}

/// The windows of each foreground app, asked one app at a time; `None`
/// when no app answered.
pub(super) fn windows_app_by_app(
    call: &dyn Fn(&[&str]) -> Result<Value, String>,
    apps: &[String],
) -> Option<Vec<desktop::Window>> {
    let mut answered = false;
    let mut windows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for app in apps {
        if !seen.insert(app.as_str()) {
            continue;
        }
        match call(&["list-windows", "--app", app, "--timeout-ms", "2000"]) {
            Ok(data) => {
                answered = true;
                windows.extend(desktop::windows(&data));
            }
            Err(error) => tracing::info!(%app, %error, "agent-desktop skipped an app's windows"),
        }
    }
    answered.then_some(windows)
}

pub(super) fn failed_round_trip(check: Check, what: &str, why: &str) -> CheckResult {
    let fix = if why.contains("PERM") || why.to_ascii_lowercase().contains("permission") {
        Fix::GuidePermissions
    } else {
        Fix::Retry
    };
    CheckResult::failed(
        check,
        format!("agent-desktop couldn't {what}: {why}"),
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
    // With Cua Driver the agent calls the guard's desktop tools (read only),
    // which serve `desktop_status` as agent-desktop did.
    let server = if cua_active() {
        crate::assistant::guard::check_server()
    } else {
        super::super::mcp_server(&[])
    };
    let Some(server) = server else {
        return CheckResult::failed(
            check,
            "No desktop driver is installed",
            Some(Fix::InstallAgentDesktop),
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
            // A private server, so the config above is its own.
            command.args(["run", "--standalone", "--format", "json", "--archived"]);
            if let Some(model) = &model {
                command.args(["--model", model]);
            }
            command.arg(status_prompt(&tool));
        }
        AssistantBackend::ClaudeApi => {
            return CheckResult::passed(
                check,
                "Agent-Desk runs agent-desktop for the Claude API itself",
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
                        "Called agent-desktop in {:.1} s; it has Accessibility and Screen Recording",
                        ran.took.as_secs_f32()
                    );
                }
                result
            }
            None => CheckResult::failed(
                check,
                "agent-desktop answered without its permissions",
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
                    format!("It never called agent-desktop. {}", ran.last_words())
                } else {
                    format!(
                        "It never called agent-desktop; it said “{}”",
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
