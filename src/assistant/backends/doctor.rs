//! `hex dev assistant-doctor` (debug builds): prints what the setup sheet
//! would find and runs its self-tests, from this process, with the checks
//! this OS has (`Check::for_backend_on`). Run from a terminal, macOS judges
//! that terminal's permissions, not Hex.app's; Windows and Linux ask for
//! none, so there it reports what Hex itself would see.

use std::path::PathBuf;

use crate::app_settings::AssistantSettings;
use crate::assistant::{computer, speech};

/// What to do beyond printing what's found: `hex dev assistant-doctor`'s
/// arguments on every OS.
#[derive(Clone, Debug, Default, clap::Args)]
pub struct Doctor {
    /// Run every self-test (asks each installed agent a tiny prompt).
    #[arg(long)]
    pub checks: bool,
    /// Capture a question's context, writing its screenshot to this PNG.
    #[arg(long)]
    pub context: Option<PathBuf>,
    /// Read this text aloud.
    #[arg(long)]
    pub say: Option<String>,
    /// Run only the hidden browser's self-tests: agent-browser directly,
    /// through Hex's browser tool, and through each ready agent (a tiny
    /// prompt each).
    #[arg(long)]
    pub browser: bool,
}

/// `hex dev` on Windows and Linux (debug builds): the assistant's doctor.
/// macOS has its own, longer `hex dev`.
#[cfg(not(target_os = "macos"))]
#[derive(clap::Subcommand)]
pub enum Dev {
    /// The assistant: find the agents and agent-desktop, and run setup's
    /// self-tests from this process.
    AssistantDoctor(Doctor),
}

#[cfg(not(target_os = "macos"))]
impl Dev {
    pub fn run(self) -> color_eyre::Result<()> {
        match self {
            Self::AssistantDoctor(doctor) => run(doctor),
        }
    }
}

pub fn run(doctor: Doctor) -> color_eyre::Result<()> {
    let os = super::os::Os::HERE;
    println!("os: {os:?}");
    let started = std::time::Instant::now();
    let found = super::detect();
    println!("detect() in {:.1} s:", started.elapsed().as_secs_f32());
    for found in &found {
        println!(
            "  {:<12} ready={:<5} version={:<10} path={} {}",
            super::name(found.backend),
            found.ready,
            found.version.as_deref().unwrap_or("-"),
            found
                .path
                .as_deref()
                .map_or("-".into(), |path| path.display().to_string()),
            found.note.as_deref().unwrap_or_default(),
        );
    }
    println!("agent_desktop(): {:?}", super::agent_desktop());
    if let Some(path) = super::search::find("agent-desktop") {
        println!(
            "agent-desktop {}: {:?}",
            super::desktop_report_command(os),
            super::desktop_call(&path, &[super::desktop_report_command(os)])
        );
    }
    for tool in super::BACKENDS
        .into_iter()
        .filter(|backend| *backend != crate::app_settings::AssistantBackend::ClaudeApi)
        .map(super::Tool::Agent)
        .chain([
            super::Tool::AgentDesktop,
            super::Tool::AgentBrowser,
            super::Tool::Lightpanda,
        ])
    {
        let plan = super::install::plan_for(os, tool, &super::Installers::here(tool));
        println!("install plan for {}: {plan:?}", tool.name());
    }
    println!(
        "browser::agent_browser(): {:?} signed_in={}",
        crate::assistant::browser::agent_browser(),
        crate::assistant::browser::signed_in()
    );
    println!(
        "computer::permissions_now(): {:?}",
        computer::permissions_now()
    );
    let path = super::agent_env()
        .into_iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, path)| path)
        .unwrap_or_default();
    println!("agent PATH: {path}");
    let voices = speech::voices_with_languages();
    println!("voices: {} installed", voices.len());

    if let Some(file) = &doctor.context {
        let started = std::time::Instant::now();
        let quick = computer::quick_context();
        println!(
            "quick_context() in {} ms: app={:?}",
            started.elapsed().as_millis(),
            quick.frontmost_app
        );
        let started = std::time::Instant::now();
        let handle = computer::context_async(true);
        println!(
            "context_async(true) returned in {} µs",
            started.elapsed().as_micros()
        );
        let context = handle.wait(std::time::Duration::from_secs(5));
        println!(
            "  waited for it: {} ms: app={:?} window={:?} selected={:?}",
            started.elapsed().as_millis(),
            context.frontmost_app,
            context.window_title,
            context
                .selected_text
                .as_deref()
                .map(|text| super::run::shorten(text, 80)),
        );
        match &context.screenshot {
            Some(shot) => {
                std::fs::write(file, &shot.png)?;
                println!(
                    "  screenshot {}×{} ({} KB) → {}",
                    shot.width,
                    shot.height,
                    shot.png.len() / 1024,
                    file.display()
                );
            }
            None => println!("  no screenshot (see the log)"),
        }
    }

    if doctor.checks || doctor.browser {
        let settings = AssistantSettings::default();
        let api = crate::app_settings::AssistantBackend::ClaudeApi;
        // This OS's own checks, in setup's order, without the agents'.
        let mut checks: Vec<super::Check> = super::Check::for_backend_on(os, api, true)
            .into_iter()
            .filter(|check| {
                matches!(
                    check,
                    super::Check::HexPermissions
                        | super::Check::DesktopPermissions
                        | super::Check::DesktopRoundTrip
                        | super::Check::BrowserRoundTrip
                        | super::Check::LightBrowser
                )
            })
            .filter(|check| {
                !doctor.browser
                    || matches!(
                        check,
                        super::Check::BrowserRoundTrip | super::Check::LightBrowser
                    )
            })
            .collect();
        if doctor.browser {
            // Hex's own MCP path, as the Claude API's tasks use it, then
            // each agent that is ready (a tiny prompt each).
            checks.push(super::Check::BrowserThroughAgent(api));
            checks.extend(
                found
                    .iter()
                    .filter(|found| found.ready && found.path.is_some())
                    .map(|found| super::Check::BrowserThroughAgent(found.backend)),
            );
        }
        for found in found.iter().filter(|_| doctor.checks) {
            let installed = found.path.is_some() || found.backend == api;
            if installed {
                checks.push(super::Check::Answers(found.backend));
                if found.backend != api {
                    checks.push(super::Check::DesktopThroughAgent(found.backend));
                }
                checks.push(super::Check::BrowserThroughAgent(found.backend));
            }
        }
        println!("self-tests:");
        super::self_tests(&checks, &settings, |result| {
            let state = match result.state {
                super::CheckState::Running => {
                    println!("  … {}", result.check.title());
                    return;
                }
                state => state,
            };
            println!(
                "  {:?} {} — {} ({:.1} s){}",
                state,
                result.check.title(),
                result.detail,
                result.took.as_secs_f32(),
                result
                    .fix
                    .map(|fix| format!(" [fix: {}]", fix.label()))
                    .unwrap_or_default(),
            );
        });
    }

    if let Some(text) = &doctor.say {
        let voice = speech::automatic_voice(text);
        println!(
            "speaking {text:?} with {}",
            voice.as_deref().unwrap_or("the system voice")
        );
        speech::speak(text, None);
        while speech::speaking() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    Ok(())
}
