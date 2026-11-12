//! Setup's one-click fixes: installing an agent, agent-desktop, or
//! agent-browser without a prompt, with each line of the installer's output
//! handed to the caller as progress, and signing in in a terminal window
//! (`sign_in`) while Hex waits for the agent to say it's signed in.
//!
//! Each install uses its project's documented method for this OS
//! ([`plan_for`]). On macOS: Homebrew when it's there, else npm when Node
//! is, else the project's own install script. On Windows and Linux: npm,
//! else Homebrew on Linux for the tools it has, else the project's script
//! (Claude Code's PowerShell installer on Windows). Without one,
//! [`install_command`] gives what to paste in a terminal. Lightpanda has no
//! Windows build; there clean browsers use Chrome. Something already
//! installed is never installed again.

mod sign_in;

use std::path::{Path, PathBuf};
use std::time::Duration;

use color_eyre::{Result, eyre::eyre};

use super::check::Fix;
use super::os::Os;
use super::{agent_command, name, run, search};
use crate::app_settings::AssistantBackend;
use crate::assistant::computer;

/// How long one install command may take (a cask, Chrome for Testing).
const INSTALL_LIMIT: Duration = Duration::from_secs(600);

/// Installs Homebrew on macOS; it asks for the person's password, so only a
/// person runs it, in Terminal.
const HOMEBREW: &str = r#"/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" && eval "$(/opt/homebrew/bin/brew shellenv 2>/dev/null || /usr/local/bin/brew shellenv)""#;

/// Why Lightpanda isn't offered on Windows.
pub const NO_LIGHTPANDA_ON_WINDOWS: &str =
    "Lightpanda has no Windows build, so clean browsers use Chrome here";

/// Something setup can install.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Agent(AssistantBackend),
    AgentDesktop,
    AgentBrowser,
    /// The light engine for clean browsers.
    Lightpanda,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Agent(backend) => name(backend),
            Self::AgentDesktop => "agent-desktop",
            Self::AgentBrowser => "agent-browser",
            Self::Lightpanda => "Lightpanda",
        }
    }

    /// The executable it puts on the PATH.
    fn program(self) -> Option<&'static str> {
        match self {
            Self::Agent(backend) => super::program(backend),
            Self::AgentDesktop => Some("agent-desktop"),
            Self::AgentBrowser => Some("agent-browser"),
            Self::Lightpanda => Some("lightpanda"),
        }
    }

    /// `brew install` arguments on `os` (none: not in Homebrew there).
    /// Linux's Homebrew has no casks, and of these tools only
    /// agent-desktop and agent-browser say it works there.
    fn brew(self, os: Os) -> &'static [&'static str] {
        match (os, self) {
            (Os::Windows, _) => &[],
            (Os::Mac, Self::Agent(AssistantBackend::ClaudeCode)) => &["--cask", "claude-code"],
            (Os::Mac, Self::Agent(AssistantBackend::Codex)) => &["--cask", "codex"],
            (Os::Mac, Self::Agent(AssistantBackend::OpenCode)) => &["anomalyco/tap/opencode-v2"],
            (_, Self::AgentDesktop) => &["agent-desktop"],
            (_, Self::AgentBrowser) => &["agent-browser"],
            _ => &[],
        }
    }

    /// The npm package (the same on every OS).
    fn npm(self) -> Option<&'static str> {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => Some("@anthropic-ai/claude-code"),
            Self::Agent(AssistantBackend::Codex) => Some("@openai/codex"),
            Self::Agent(AssistantBackend::OpenCode) => Some("@opencode/cli"),
            Self::Agent(AssistantBackend::ClaudeApi) | Self::Lightpanda => None,
            Self::AgentDesktop => Some("agent-desktop"),
            Self::AgentBrowser => Some("agent-browser"),
        }
    }

    /// The project's own install script on `os`, as its docs give it: for
    /// `bash` on macOS and Linux, for PowerShell on Windows.
    fn script(self, os: Os) -> Option<&'static str> {
        match (os, self) {
            (Os::Windows, Self::Agent(AssistantBackend::ClaudeCode)) => {
                Some("irm https://claude.ai/install.ps1 | iex")
            }
            (Os::Windows, _) => None,
            (_, Self::Agent(AssistantBackend::ClaudeCode)) => {
                Some("curl -fsSL https://claude.ai/install.sh | bash")
            }
            (_, Self::Agent(AssistantBackend::Codex)) => {
                Some("curl -fsSL https://chatgpt.com/codex/install.sh | sh")
            }
            (_, Self::Agent(AssistantBackend::OpenCode)) => {
                Some("curl -fsSL https://opencode.ai/v2/install | bash")
            }
            // Its installer checks the download's checksum with jq (part
            // of macOS since 15); given no version it takes the nightly, so
            // it gets the latest release's tag.
            (_, Self::Lightpanda) => Some(
                "curl -fsSL https://pkg.lightpanda.io/install.sh | bash -s \"$(curl -fsSL \
                 https://api.github.com/repos/lightpanda-io/browser/releases/latest | jq -r \
                 .tag_name)\"",
            ),
            _ => None,
        }
    }

    /// What `fix` installs.
    pub fn of(fix: Fix) -> Option<Self> {
        match fix {
            Fix::Install(AssistantBackend::ClaudeApi) => None,
            Fix::Install(backend) => Some(Self::Agent(backend)),
            Fix::InstallAgentDesktop => Some(Self::AgentDesktop),
            Fix::InstallAgentBrowser => Some(Self::AgentBrowser),
            Fix::InstallLightpanda => Some(Self::Lightpanda),
            _ => None,
        }
    }
}

/// What this computer has for installing `tool`, and whether it's there
/// already.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Installers {
    pub brew: Option<PathBuf>,
    pub npm: Option<PathBuf>,
    pub installed: bool,
    /// agent-browser's Chrome is downloaded.
    pub browser_ready: bool,
    /// `jq`, which Lightpanda's installer needs (macOS has it).
    pub jq: bool,
}

impl Installers {
    /// As found now (asks the login shell the first time; off the UI
    /// thread).
    pub fn here(tool: Tool) -> Self {
        let installed = tool.program().and_then(search::find);
        let browser_ready = tool == Tool::AgentBrowser
            && installed.is_some()
            && crate::assistant::browser::agent_browser().browser_ready;
        Self {
            brew: brew(),
            // npm runs on node; without it, npm can't install anything.
            npm: search::find("node").and_then(|_| search::find("npm")),
            installed: installed.is_some(),
            browser_ready,
            jq: Os::HERE == Os::Mac || search::find("jq").is_some(),
        }
    }
}

fn brew() -> Option<PathBuf> {
    let known: &[&str] = match Os::HERE {
        Os::Mac => &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"],
        Os::Linux => &["/home/linuxbrew/.linuxbrew/bin/brew"],
        Os::Windows => return None,
    };
    known
        .iter()
        .map(PathBuf::from)
        .find(|path| search::is_executable(path))
        .or_else(|| search::find("brew"))
}

/// One command of an install: an executable's path, or a name found on the
/// PATH once the steps before it ran.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
    /// What a person pastes instead, when that isn't the command itself
    /// (a PowerShell installer's own line).
    pub paste: Option<String>,
}

impl Step {
    fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            paste: None,
        }
    }

    /// `script` run by `os`'s shell for installers: bash on macOS and
    /// Linux, PowerShell on Windows.
    fn script(os: Os, script: &str) -> Self {
        match os {
            Os::Mac => Self::new("/bin/bash", &["-c", script]),
            Os::Linux => Self::new("bash", &["-c", script]),
            Os::Windows => Self {
                paste: Some(script.to_owned()),
                ..Self::new(
                    "powershell",
                    &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-Command",
                        script,
                    ],
                )
            },
        }
    }

    /// As typed in `os`'s terminal (PowerShell on Windows).
    pub fn command_line(&self, os: Os) -> String {
        if let Some(paste) = &self.paste {
            return paste.clone();
        }
        let line = std::iter::once(&self.program)
            .chain(&self.args)
            .map(|word| quote(os, word))
            .collect::<Vec<_>>()
            .join(" ");
        // PowerShell runs a quoted program only after `&`.
        if os == Os::Windows && quote(os, &self.program) != self.program {
            format!("& {line}")
        } else {
            line
        }
    }
}

fn quote(os: Os, word: &str) -> String {
    let safe = if os == Os::Windows {
        "-_./:@=+\\"
    } else {
        "-_./:@=+"
    };
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || safe.contains(c));
    if plain {
        word.to_owned()
    } else if os == Os::Windows {
        format!("'{}'", word.replace('\'', "''"))
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// How `tool` would be installed here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Plan {
    /// Already installed (and agent-browser's Chrome downloaded).
    Ready,
    Run(Vec<Step>),
    /// Hex can't install it by itself: why, and what the person can paste
    /// in a terminal.
    Missing {
        why: String,
        command: String,
    },
    /// It doesn't exist for this OS (Lightpanda on Windows): why.
    Unavailable(String),
}

/// The install for `tool` on this OS given what `here` has.
pub fn plan(tool: Tool, here: &Installers) -> Plan {
    plan_for(Os::HERE, tool, here)
}

/// The install for `tool` on `os` given what `here` has: on macOS
/// Homebrew, else npm, else the project's script; elsewhere npm, else
/// Homebrew (Linux), else the script. agent-browser then downloads its
/// Chrome.
pub fn plan_for(os: Os, tool: Tool, here: &Installers) -> Plan {
    if os == Os::Windows && tool == Tool::Lightpanda {
        return Plan::Unavailable(NO_LIGHTPANDA_ON_WINDOWS.into());
    }
    let chrome = Step::new("agent-browser", &["install"]);
    if here.installed {
        return match tool {
            Tool::AgentBrowser if !here.browser_ready => Plan::Run(vec![chrome]),
            _ => Plan::Ready,
        };
    }
    let brew = here
        .brew
        .as_ref()
        .filter(|_| !tool.brew(os).is_empty())
        .map(|brew| {
            let mut args = vec!["install"];
            args.extend(tool.brew(os));
            Step::new(brew.to_string_lossy(), &args)
        });
    let npm = here
        .npm
        .as_ref()
        .zip(tool.npm())
        .map(|(npm, package)| npm_step(&npm.to_string_lossy(), package));
    let script = tool
        .script(os)
        .filter(|_| tool != Tool::Lightpanda || here.jq)
        .map(|script| Step::script(os, script));
    let package = match os {
        Os::Mac => brew.or(npm).or(script),
        Os::Linux | Os::Windows => npm.or(brew).or(script),
    };
    let Some(package) = package else {
        return missing(os, tool);
    };
    let mut steps = vec![package];
    if tool == Tool::AgentBrowser {
        steps.push(chrome);
    }
    Plan::Run(steps)
}

fn npm_step(npm: &str, package: &str) -> Step {
    Step::new(npm, &["install", "-g", "--no-fund", "--no-audit", package])
}

/// Why Hex can't install `tool` on `os` by itself, and what to paste.
fn missing(os: Os, tool: Tool) -> Plan {
    let then = if tool == Tool::AgentBrowser {
        " && agent-browser install"
    } else {
        ""
    };
    if os == Os::Mac {
        return Plan::Missing {
            why: format!(
                "Homebrew isn't installed, so Hex can't install {} by itself. Install \
                 Homebrew from brew.sh, or paste this in Terminal",
                tool.name()
            ),
            command: format!(
                "{HOMEBREW} && brew install {}{then}",
                tool.brew(os).join(" ")
            ),
        };
    }
    if tool == Tool::Lightpanda {
        return Plan::Missing {
            why: "Lightpanda's installer needs jq. Install jq, then try again, or paste this in \
                  a terminal once it's there"
                .into(),
            command: tool.script(os).unwrap_or_default().to_owned(),
        };
    }
    let npm = npm_step("npm", tool.npm().unwrap_or_default()).command_line(os);
    // PowerShell 5 has no `&&`.
    let then = if os == Os::Windows {
        then.replace(" && ", "; ")
    } else {
        then.to_owned()
    };
    let get_node = match os {
        Os::Windows => "Install Node.js from nodejs.org, then paste this in PowerShell",
        _ => {
            "Install Node.js and npm with your system's package manager, then paste this in a \
              terminal"
        }
    };
    Plan::Missing {
        why: format!(
            "Node.js isn't installed, so Hex can't install {} by itself. {get_node}",
            tool.name()
        ),
        command: format!("{npm}{then}"),
    }
}

/// What a person could paste in a terminal to do what `fix` installs, for
/// setup's "Copy install command"; `None` when it installs nothing (or
/// it's installed, or doesn't exist for this OS).
pub fn install_command(fix: Fix) -> Option<String> {
    let tool = Tool::of(fix)?;
    match plan(tool, &Installers::here(tool)) {
        Plan::Ready | Plan::Unavailable(_) => None,
        Plan::Run(steps) => Some(
            steps
                .iter()
                .map(|step| step.command_line(Os::HERE))
                .collect::<Vec<_>>()
                .join(if Os::HERE == Os::Windows {
                    "; "
                } else {
                    " && "
                }),
        ),
        Plan::Missing { command, .. } => Some(command),
    }
}

/// Installs `tool`, handing `progress` each line of output, then finds it
/// again. Blocks for minutes at most.
pub(super) fn install(tool: Tool, progress: &mut dyn FnMut(&str)) -> Result<()> {
    let steps = match plan(tool, &Installers::here(tool)) {
        Plan::Ready => {
            progress(&format!("{} is already installed", tool.name()));
            return Ok(());
        }
        Plan::Missing { why, .. } | Plan::Unavailable(why) => return Err(eyre!("{why}")),
        Plan::Run(steps) => steps,
    };
    for step in &steps {
        progress(&format!("$ {}", step.command_line(Os::HERE)));
        run_step(step, progress)?;
        super::refresh();
    }
    let program = tool.program().unwrap_or_default();
    let path = search::find(program).ok_or_else(|| {
        eyre!(
            "{} was installed, but Hex can't find `{program}`",
            tool.name()
        )
    })?;
    let version = super::version_of(&path).ok().flatten().unwrap_or_default();
    progress(format!("Installed {} {version}", tool.name()).trim_end());
    if tool == Tool::AgentBrowser && !crate::assistant::browser::agent_browser().browser_ready {
        return Err(eyre!("agent-browser is installed, but its Chrome isn't"));
    }
    Ok(())
}

fn run_step(step: &Step, progress: &mut dyn FnMut(&str)) -> Result<()> {
    let program = if Path::new(&step.program).is_absolute() {
        PathBuf::from(&step.program)
    } else {
        search::find(&step.program).ok_or_else(|| eyre!("Hex can't find `{}`", step.program))?
    };
    let mut command = agent_command(&program);
    command
        .args(&step.args)
        .env("NONINTERACTIVE", "1")
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .env("npm_config_yes", "true");
    let ran = run::streamed(command, INSTALL_LIMIT, progress)
        .map_err(|error| eyre!("`{}` didn't start: {error}", step.program))?;
    if ran.timed_out() {
        return Err(eyre!(
            "`{}` took more than {} minutes",
            step.command_line(Os::HERE),
            INSTALL_LIMIT.as_secs() / 60
        ));
    }
    if !ran.success() {
        return Err(eyre!(
            "`{}` failed: {}",
            step.command_line(Os::HERE),
            ran.last_words()
        ));
    }
    Ok(())
}

/// Does what `fix` offers. Blocks while installing or waiting for a
/// sign-in; call it off the UI thread, except [`Fix::OpenSettings`], which
/// shows Hex's guide only from the main thread. Run the check again
/// afterwards.
pub fn apply_fix(fix: Fix) -> Result<()> {
    apply_fix_with(fix, |_| {})
}

/// [`apply_fix`], handing `progress` what it's doing as it goes: an
/// installer's output line by line, "Waiting for you to sign in…".
pub fn apply_fix_with(fix: Fix, mut progress: impl FnMut(&str)) -> Result<()> {
    let progress: &mut dyn FnMut(&str) = &mut progress;
    match fix {
        Fix::Install(AssistantBackend::ClaudeApi) => {
            super::os::open("https://console.anthropic.com/settings/keys")
                .map_err(|error| eyre!("{error}"))
        }
        Fix::Install(backend) => install(Tool::Agent(backend), progress),
        Fix::InstallAgentDesktop => install(Tool::AgentDesktop, progress),
        Fix::InstallAgentBrowser => install(Tool::AgentBrowser, progress),
        Fix::InstallLightpanda => install(Tool::Lightpanda, progress),
        Fix::UseApiKey(backend) => super::providers::use_api_key(backend, progress),
        Fix::SignIn(backend) => {
            sign_in::open(backend)?;
            progress(&format!(
                "Sign in to {} in the {} Hex opened…",
                name(backend),
                sign_in::window_name(Os::HERE)
            ));
            sign_in::wait(backend, progress)
        }
        Fix::StartOpenCode => {
            let path = super::executable(AssistantBackend::OpenCode)
                .ok_or_else(|| eyre!("OpenCode isn't installed"))?;
            let mut command = agent_command(&path);
            command.args(["service", "start"]);
            let ran = run::bounded(command, Duration::from_secs(30))?;
            if ran.success() {
                Ok(())
            } else {
                Err(eyre!("OpenCode didn't start: {}", ran.last_words()))
            }
        }
        Fix::AddApiKey | Fix::Retry => Ok(()),
        Fix::GuidePermissions => {
            super::guide_permissions();
            Ok(())
        }
        Fix::OpenSettings(permission) => {
            computer::open_settings(permission);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
