//! Setup's one-click fixes: installing an agent, agent-desktop, or
//! agent-browser without a prompt, with each line of the installer's output
//! handed to the caller as progress, and signing in through Terminal while
//! Hex waits for the agent to say it's signed in.
//!
//! Each install uses its project's documented method: Homebrew when it's
//! there, else npm when Node is, else the project's own install script (the
//! agents have one; agent-desktop and agent-browser need Homebrew or npm,
//! and without either [`install_command`] gives what to paste in Terminal).
//! Something already installed is never installed again.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::eyre};

use super::check::Fix;
use super::{agent_command, name, run, search};
use crate::app_settings::AssistantBackend;
use crate::assistant::computer;

/// How long one install command may take (a cask, Chrome for Testing).
const INSTALL_LIMIT: Duration = Duration::from_secs(600);

/// How long Hex waits for a sign-in in Terminal.
const SIGN_IN_LIMIT: Duration = Duration::from_secs(600);

/// Installs Homebrew; it asks for the person's password, so only a person
/// runs it, in Terminal.
const HOMEBREW: &str = r#"/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" && eval "$(/opt/homebrew/bin/brew shellenv 2>/dev/null || /usr/local/bin/brew shellenv)""#;

/// Something setup can install.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Agent(AssistantBackend),
    AgentDesktop,
    AgentBrowser,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Agent(backend) => name(backend),
            Self::AgentDesktop => "agent-desktop",
            Self::AgentBrowser => "agent-browser",
        }
    }

    /// The executable it puts on the PATH.
    fn program(self) -> Option<&'static str> {
        match self {
            Self::Agent(backend) => super::program(backend),
            Self::AgentDesktop => Some("agent-desktop"),
            Self::AgentBrowser => Some("agent-browser"),
        }
    }

    /// `brew install` arguments.
    fn brew(self) -> &'static [&'static str] {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => &["--cask", "claude-code"],
            Self::Agent(AssistantBackend::Codex) => &["--cask", "codex"],
            Self::Agent(AssistantBackend::OpenCode) => &["anomalyco/tap/opencode-v2"],
            Self::Agent(AssistantBackend::ClaudeApi) => &[],
            Self::AgentDesktop => &["agent-desktop"],
            Self::AgentBrowser => &["agent-browser"],
        }
    }

    /// The npm package.
    fn npm(self) -> Option<&'static str> {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => Some("@anthropic-ai/claude-code"),
            Self::Agent(AssistantBackend::Codex) => Some("@openai/codex"),
            Self::Agent(AssistantBackend::OpenCode) => Some("@opencode/cli"),
            Self::Agent(AssistantBackend::ClaudeApi) => None,
            Self::AgentDesktop => Some("agent-desktop"),
            Self::AgentBrowser => Some("agent-browser"),
        }
    }

    /// The project's own install script, as its docs give it.
    fn script(self) -> Option<&'static str> {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => {
                Some("curl -fsSL https://claude.ai/install.sh | bash")
            }
            Self::Agent(AssistantBackend::Codex) => {
                Some("curl -fsSL https://chatgpt.com/codex/install.sh | sh")
            }
            Self::Agent(AssistantBackend::OpenCode) => {
                Some("curl -fsSL https://opencode.ai/v2/install | bash")
            }
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
            _ => None,
        }
    }
}

/// What this Mac has for installing `tool`, and whether it's there already.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Installers {
    pub brew: Option<PathBuf>,
    pub npm: Option<PathBuf>,
    pub installed: bool,
    /// agent-browser's Chrome is downloaded.
    pub browser_ready: bool,
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
        }
    }
}

fn brew() -> Option<PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .map(PathBuf::from)
        .into_iter()
        .find(|path| search::is_executable(path))
        .or_else(|| search::find("brew"))
}

/// One command of an install: an executable's path, or a name found on the
/// PATH once the steps before it ran.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
}

impl Step {
    fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    /// As typed in a shell.
    pub fn command_line(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|word| quote(word))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@=+".contains(c));
    if plain {
        word.to_owned()
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
    /// in Terminal.
    Missing {
        why: String,
        command: String,
    },
}

/// The install for `tool` given what `here` has: Homebrew, else npm, else
/// the project's script; agent-browser then downloads its Chrome.
pub fn plan(tool: Tool, here: &Installers) -> Plan {
    let chrome = Step::new("agent-browser", &["install"]);
    if here.installed {
        return match tool {
            Tool::AgentBrowser if !here.browser_ready => Plan::Run(vec![chrome]),
            _ => Plan::Ready,
        };
    }
    let package = if let Some(brew) = &here.brew {
        let mut args = vec!["install"];
        args.extend(tool.brew());
        Step::new(brew.to_string_lossy(), &args)
    } else if let (Some(npm), Some(package)) = (&here.npm, tool.npm()) {
        Step::new(
            npm.to_string_lossy(),
            &["install", "-g", "--no-fund", "--no-audit", package],
        )
    } else if let Some(script) = tool.script() {
        Step::new("/bin/bash", &["-c", script])
    } else {
        let mut install = format!("brew install {}", tool.brew().join(" "));
        if tool == Tool::AgentBrowser {
            install.push_str(" && agent-browser install");
        }
        return Plan::Missing {
            why: format!(
                "Homebrew isn't installed, so Hex can't install {} by itself. Install \
                 Homebrew from brew.sh, or paste this in Terminal",
                tool.name()
            ),
            command: format!("{HOMEBREW} && {install}"),
        };
    };
    let mut steps = vec![package];
    if tool == Tool::AgentBrowser {
        steps.push(chrome);
    }
    Plan::Run(steps)
}

/// What a person could paste in Terminal to do what `fix` installs, for
/// setup's "Copy install command"; `None` when it installs nothing (or
/// it's installed).
pub fn install_command(fix: Fix) -> Option<String> {
    let tool = Tool::of(fix)?;
    match plan(tool, &Installers::here(tool)) {
        Plan::Ready => None,
        Plan::Run(steps) => Some(
            steps
                .iter()
                .map(Step::command_line)
                .collect::<Vec<_>>()
                .join(" && "),
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
        Plan::Missing { why, .. } => return Err(eyre!("{why}")),
        Plan::Run(steps) => steps,
    };
    for step in &steps {
        progress(&format!("$ {}", step.command_line()));
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
            step.command_line(),
            INSTALL_LIMIT.as_secs() / 60
        ));
    }
    if !ran.success() {
        return Err(eyre!(
            "`{}` failed: {}",
            step.command_line(),
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
            open_url("https://console.anthropic.com/settings/keys")
        }
        Fix::Install(backend) => install(Tool::Agent(backend), progress),
        Fix::InstallAgentDesktop => install(Tool::AgentDesktop, progress),
        Fix::InstallAgentBrowser => install(Tool::AgentBrowser, progress),
        Fix::SignIn(backend) => {
            sign_in(backend)?;
            progress(&format!(
                "Sign in to {} in the Terminal window Hex opened…",
                name(backend)
            ));
            wait_for_sign_in(backend, progress)
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

fn open_url(url: &str) -> Result<()> {
    let status = crate::desktop::background_process::command("/usr/bin/open")
        .arg(url)
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| eyre!("Couldn't open {url}"))
}

/// The sign-in command for `backend`, after its executable.
fn sign_in_arguments(backend: AssistantBackend) -> Option<&'static str> {
    match backend {
        AssistantBackend::ClaudeCode => Some(" auth login"),
        AssistantBackend::Codex => Some(" login"),
        AssistantBackend::OpenCode => Some(" auth login"),
        AssistantBackend::ClaudeApi => None,
    }
}

/// Terminal, running the agent's sign-in: a `.command` file Terminal opens.
fn sign_in(backend: AssistantBackend) -> Result<()> {
    let path =
        super::executable(backend).ok_or_else(|| eyre!("{} isn't installed", name(backend)))?;
    let arguments = sign_in_arguments(backend)
        .ok_or_else(|| eyre!("The Claude API needs a key, not a sign-in"))?;
    let folder = std::env::temp_dir().join("hex-assistant");
    std::fs::create_dir_all(&folder)?;
    let script = folder.join(format!("Sign in to {}.command", name(backend)));
    let quoted = format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"));
    std::fs::write(
        &script,
        format!("#!/bin/zsh -l\nclear\nexec {quoted}{arguments}\n"),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    }
    let status = crate::desktop::background_process::command("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(&script)
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| eyre!("Couldn't open Terminal"))
}

/// Asks the agent every few seconds whether it's signed in, for ten
/// minutes at most.
fn wait_for_sign_in(backend: AssistantBackend, progress: &mut dyn FnMut(&str)) -> Result<()> {
    let deadline = Instant::now() + SIGN_IN_LIMIT;
    let mut said_waiting = false;
    loop {
        std::thread::sleep(Duration::from_secs(3));
        let found = super::find(backend);
        if found.ready {
            progress(&format!("Signed in to {}", name(backend)));
            return Ok(());
        }
        if !said_waiting {
            progress("Waiting for you to finish signing in…");
            said_waiting = true;
        }
        if Instant::now() >= deadline {
            return Err(eyre!(
                "{} isn't signed in yet. Finish in Terminal, then try again",
                name(backend)
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here(brew: bool, npm: bool) -> Installers {
        Installers {
            brew: brew.then(|| PathBuf::from("/opt/homebrew/bin/brew")),
            npm: npm.then(|| PathBuf::from("/Users/me/.fnm/bin/npm")),
            installed: false,
            browser_ready: false,
        }
    }

    fn lines(plan: Plan) -> Vec<String> {
        match plan {
            Plan::Run(steps) => steps.iter().map(Step::command_line).collect(),
            other => panic!("not a run: {other:?}"),
        }
    }

    #[test]
    fn homebrew_comes_first() {
        let brew = here(true, true);
        let claude = Tool::Agent(AssistantBackend::ClaudeCode);
        assert_eq!(
            lines(plan(claude, &brew)),
            ["/opt/homebrew/bin/brew install --cask claude-code"]
        );
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::Codex), &brew)),
            ["/opt/homebrew/bin/brew install --cask codex"]
        );
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::OpenCode), &brew)),
            ["/opt/homebrew/bin/brew install anomalyco/tap/opencode-v2"]
        );
        assert_eq!(
            lines(plan(Tool::AgentDesktop, &brew)),
            ["/opt/homebrew/bin/brew install agent-desktop"]
        );
        assert_eq!(
            lines(plan(Tool::AgentBrowser, &brew)),
            [
                "/opt/homebrew/bin/brew install agent-browser",
                "agent-browser install"
            ]
        );
    }

    #[test]
    fn npm_is_next_when_node_is_there() {
        let npm = here(false, true);
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::ClaudeCode), &npm)),
            ["/Users/me/.fnm/bin/npm install -g --no-fund --no-audit @anthropic-ai/claude-code"]
        );
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::OpenCode), &npm)),
            ["/Users/me/.fnm/bin/npm install -g --no-fund --no-audit @opencode/cli"]
        );
        assert_eq!(
            lines(plan(Tool::AgentBrowser, &npm))[1],
            "agent-browser install"
        );
    }

    #[test]
    fn agents_fall_back_to_their_own_scripts() {
        let bare = here(false, false);
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::ClaudeCode), &bare)),
            ["/bin/bash -c 'curl -fsSL https://claude.ai/install.sh | bash'"]
        );
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::Codex), &bare)),
            ["/bin/bash -c 'curl -fsSL https://chatgpt.com/codex/install.sh | sh'"]
        );
        assert_eq!(
            lines(plan(Tool::Agent(AssistantBackend::OpenCode), &bare)),
            ["/bin/bash -c 'curl -fsSL https://opencode.ai/v2/install | bash'"]
        );
    }

    #[test]
    fn without_homebrew_or_npm_the_tools_say_what_to_paste() {
        let Plan::Missing { why, command } = plan(Tool::AgentBrowser, &here(false, false)) else {
            panic!("expected Missing");
        };
        assert!(why.starts_with("Homebrew isn't installed"), "{why}");
        assert!(command.starts_with("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""));
        assert!(command.ends_with("&& brew install agent-browser && agent-browser install"));
    }

    #[test]
    fn nothing_installed_is_installed_again() {
        let mut installed = here(true, true);
        installed.installed = true;
        assert_eq!(
            plan(Tool::Agent(AssistantBackend::Codex), &installed),
            Plan::Ready
        );
        assert_eq!(plan(Tool::AgentDesktop, &installed), Plan::Ready);
        // agent-browser without its Chrome only downloads Chrome.
        assert_eq!(
            lines(plan(Tool::AgentBrowser, &installed)),
            ["agent-browser install"]
        );
        installed.browser_ready = true;
        assert_eq!(plan(Tool::AgentBrowser, &installed), Plan::Ready);
    }

    #[test]
    fn each_fix_names_what_it_installs() {
        assert_eq!(
            Tool::of(Fix::Install(AssistantBackend::Codex)),
            Some(Tool::Agent(AssistantBackend::Codex))
        );
        assert_eq!(Tool::of(Fix::Install(AssistantBackend::ClaudeApi)), None);
        assert_eq!(Tool::of(Fix::InstallAgentBrowser), Some(Tool::AgentBrowser));
        assert_eq!(Tool::of(Fix::Retry), None);
        assert_eq!(
            sign_in_arguments(AssistantBackend::ClaudeCode),
            Some(" auth login")
        );
        assert_eq!(sign_in_arguments(AssistantBackend::ClaudeApi), None);
    }
}
