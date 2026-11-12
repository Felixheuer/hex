//! Setup's one-click fixes: installing an agent, agent-desktop, or
//! agent-browser without a prompt, with each line of the installer's output
//! handed to the caller as progress, and signing in through a terminal
//! window while Hex waits for the agent to say it's signed in.
//!
//! Each install uses its project's documented method, per system:
//!
//! - macOS: Homebrew when it's there, else npm when Node is, else the
//!   project's own install script.
//! - Linux: npm, else the project's install script (run with bash), else
//!   Linuxbrew when someone has it. Claude Code's native script comes first:
//!   its npm package is deprecated.
//! - Windows: WinGet where the project publishes a package (Claude Code),
//!   else npm, else the project's PowerShell script. Lightpanda has no
//!   Windows build.
//!
//! agent-desktop and agent-browser need npm (or Homebrew on macOS); without
//! it [`install_command`] gives what to paste in a terminal. Something
//! already installed is never installed again.

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

/// The system an install is planned for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Os {
    Mac,
    Linux,
    Windows,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    /// What the person pastes a command into.
    fn terminal(self) -> &'static str {
        match self {
            Self::Mac => "Terminal",
            Self::Linux => "a terminal",
            Self::Windows => "PowerShell",
        }
    }

    /// Between the commands of a pasted line: Windows PowerShell 5 has no
    /// `&&`.
    fn then(self) -> &'static str {
        match self {
            Self::Windows => "; ",
            _ => " && ",
        }
    }
}

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

    /// `brew install` arguments (none: not in Homebrew).
    fn brew(self) -> &'static [&'static str] {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => &["--cask", "claude-code"],
            Self::Agent(AssistantBackend::Codex) => &["--cask", "codex"],
            Self::Agent(AssistantBackend::OpenCode) => &["anomalyco/tap/opencode-v2"],
            Self::Agent(AssistantBackend::ClaudeApi) | Self::Lightpanda => &[],
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
            Self::Agent(AssistantBackend::ClaudeApi) | Self::Lightpanda => None,
            Self::AgentDesktop => Some("agent-desktop"),
            Self::AgentBrowser => Some("agent-browser"),
        }
    }

    /// The WinGet package id, where the project publishes one.
    fn winget(self) -> Option<&'static str> {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => Some("Anthropic.ClaudeCode"),
            _ => None,
        }
    }

    /// The project's own PowerShell install script (Windows).
    fn powershell(self) -> Option<&'static str> {
        match self {
            Self::Agent(AssistantBackend::ClaudeCode) => {
                Some("irm https://claude.ai/install.ps1 | iex")
            }
            Self::Agent(AssistantBackend::Codex) => {
                Some("irm https://chatgpt.com/codex/install.ps1 | iex")
            }
            _ => None,
        }
    }

    /// Whether `os` has a build of it.
    pub fn available_on(self, os: Os) -> bool {
        !(self == Self::Lightpanda && os == Os::Windows)
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
            // Its installer checks the download's checksum (with jq, part
            // of macOS since 15); given no version it takes the nightly, so
            // it gets the latest release's tag.
            Self::Lightpanda => Some(
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
    /// Windows' package manager.
    pub winget: Option<PathBuf>,
    /// Windows PowerShell, for a project's `install.ps1`.
    pub powershell: Option<PathBuf>,
    /// Where a global npm install goes when npm's own folder belongs to the
    /// system (Linux's packaged npm): `~/.local`, whose `bin` Hex searches.
    pub npm_prefix: Option<PathBuf>,
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
        let windows = cfg!(windows);
        // npm runs on node; without it, npm can't install anything.
        let npm = search::find("node").and_then(|_| search::find("npm"));
        let system_npm =
            cfg!(target_os = "linux") && npm.as_ref().is_some_and(|npm| npm.starts_with("/usr/"));
        Self {
            brew: brew(),
            npm_prefix: system_npm
                .then(|| crate::app_paths::home_dir().map(|home| home.join(".local")))
                .flatten(),
            npm,
            winget: windows.then(|| search::find("winget")).flatten(),
            powershell: windows.then(|| search::find("powershell")).flatten(),
            installed: installed.is_some(),
            browser_ready,
        }
    }
}

/// Homebrew (macOS), or Linuxbrew; never on Windows.
fn brew() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    [
        "/opt/homebrew/bin/brew",
        "/usr/local/bin/brew",
        "/home/linuxbrew/.linuxbrew/bin/brew",
    ]
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

    /// As typed in this system's shell.
    pub fn command_line(&self) -> String {
        self.command_line_for(Os::current())
    }

    /// As typed in `os`'s shell: a POSIX shell, or PowerShell on Windows.
    pub fn command_line_for(&self, os: Os) -> String {
        let words = std::iter::once(&self.program)
            .chain(&self.args)
            .map(|word| quote(word, os))
            .collect::<Vec<_>>()
            .join(" ");
        // PowerShell runs a quoted program only after `&`.
        if os == Os::Windows && words.starts_with('\'') {
            format!("& {words}")
        } else {
            words
        }
    }
}

fn quote(word: &str, os: Os) -> String {
    let plain_chars = if os == Os::Windows {
        "-_./:=+\\"
    } else {
        "-_./:@=+"
    };
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || plain_chars.contains(c));
    if plain {
        word.to_owned()
    } else if os == Os::Windows {
        // PowerShell's single quotes take everything literally; a quote
        // inside is doubled.
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
    /// in Terminal.
    Missing {
        why: String,
        command: String,
    },
}

/// The install for `tool` given what `here` has, on this system.
pub fn plan(tool: Tool, here: &Installers) -> Plan {
    plan_for(Os::current(), tool, here)
}

/// The install for `tool` on `os` given what `here` has; agent-browser then
/// downloads its Chrome.
pub fn plan_for(os: Os, tool: Tool, here: &Installers) -> Plan {
    let chrome = Step::new("agent-browser", &["install"]);
    if !tool.available_on(os) {
        return Plan::Missing {
            why: format!("{} has no Windows version", tool.name()),
            command: String::new(),
        };
    }
    if here.installed {
        return match tool {
            Tool::AgentBrowser if !here.browser_ready => Plan::Run(vec![chrome]),
            _ => Plan::Ready,
        };
    }
    let package = match os {
        Os::Mac => mac_step(tool, here),
        Os::Linux => linux_step(tool, here),
        Os::Windows => windows_step(tool, here),
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

fn brew_step(tool: Tool, brew: &Path) -> Step {
    let mut args = vec!["install"];
    args.extend(tool.brew());
    Step::new(brew.to_string_lossy(), &args)
}

fn npm_step(npm: &Path, package: &str, here: &Installers) -> Step {
    let prefix = here
        .npm_prefix
        .as_ref()
        .map(|prefix| prefix.to_string_lossy().into_owned());
    let mut args = vec!["install", "-g"];
    if let Some(prefix) = &prefix {
        args.extend(["--prefix", prefix]);
    }
    args.extend(["--no-fund", "--no-audit", package]);
    Step::new(npm.to_string_lossy(), &args)
}

/// Homebrew, else npm, else the project's script.
fn mac_step(tool: Tool, here: &Installers) -> Option<Step> {
    if let Some(brew) = here.brew.as_ref().filter(|_| !tool.brew().is_empty()) {
        Some(brew_step(tool, brew))
    } else if let (Some(npm), Some(package)) = (&here.npm, tool.npm()) {
        Some(npm_step(npm, package, here))
    } else {
        tool.script()
            .map(|script| Step::new("/bin/bash", &["-c", script]))
    }
}

/// npm, else the project's script, else a Linuxbrew formula (casks are
/// macOS only). Claude Code's own script first: its npm package is
/// deprecated.
fn linux_step(tool: Tool, here: &Installers) -> Option<Step> {
    let script = || {
        tool.script()
            .map(|script| Step::new("bash", &["-c", script]))
    };
    if tool == Tool::Agent(AssistantBackend::ClaudeCode)
        && let Some(step) = script()
    {
        return Some(step);
    }
    let formula = tool.brew().first().is_some_and(|first| *first != "--cask");
    if let (Some(npm), Some(package)) = (&here.npm, tool.npm()) {
        Some(npm_step(npm, package, here))
    } else if let Some(step) = script() {
        Some(step)
    } else {
        here.brew
            .as_ref()
            .filter(|_| formula)
            .map(|brew| brew_step(tool, brew))
    }
}

/// WinGet where the project publishes a package, else npm (Claude Code's is
/// deprecated), else the project's PowerShell script.
fn windows_step(tool: Tool, here: &Installers) -> Option<Step> {
    if let (Some(winget), Some(id)) = (&here.winget, tool.winget()) {
        return Some(Step::new(
            winget.to_string_lossy(),
            &[
                "install",
                "--id",
                id,
                "--exact",
                "--source",
                "winget",
                "--silent",
                "--disable-interactivity",
                "--accept-source-agreements",
                "--accept-package-agreements",
            ],
        ));
    }
    let npm_ok = tool != Tool::Agent(AssistantBackend::ClaudeCode);
    if let (Some(npm), Some(package), true) = (&here.npm, tool.npm(), npm_ok) {
        return Some(npm_step(npm, package, here));
    }
    let powershell = here.powershell.as_ref().map_or_else(
        || "powershell".to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );
    tool.powershell().map(|script| {
        Step::new(
            powershell,
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ],
        )
    })
}

/// Why Hex can't install `tool` by itself on `os`, and what to paste.
fn missing(os: Os, tool: Tool) -> Plan {
    let then = os.then();
    let chrome = if tool == Tool::AgentBrowser {
        format!("{then}agent-browser install")
    } else {
        String::new()
    };
    let npm = |prefix: &str| {
        tool.npm()
            .map(|package| format!("npm install -g {prefix}{package}"))
            .unwrap_or_default()
    };
    match os {
        Os::Mac => Plan::Missing {
            why: format!(
                "Homebrew isn't installed, so Hex can't install {} by itself. Install \
                 Homebrew from brew.sh, or paste this in Terminal",
                tool.name()
            ),
            command: format!(
                "{HOMEBREW} && brew install {}{chrome}",
                tool.brew().join(" ")
            ),
        },
        Os::Linux => Plan::Missing {
            why: format!(
                "Node.js and npm aren't installed, so Hex can't install {} by itself. \
                 Install them from your distribution's packages (for example \
                 `sudo apt install nodejs npm`), or paste this in a terminal",
                tool.name()
            ),
            command: format!(
                "sudo apt install -y nodejs npm && {}{chrome}",
                npm("--prefix ~/.local ")
            ),
        },
        Os::Windows => Plan::Missing {
            why: format!(
                "Node.js isn't installed, so Hex can't install {} by itself. Install \
                 Node.js from nodejs.org, or paste this in PowerShell",
                tool.name()
            ),
            command: format!(
                "winget install --id OpenJS.NodeJS.LTS --exact; $env:Path = \
                 [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + \
                 [Environment]::GetEnvironmentVariable('Path','User'); {}{chrome}",
                npm("")
            ),
        },
    }
}

/// What a person could paste in a terminal to do what `fix` installs, for
/// setup's "Copy install command"; `None` when it installs nothing (or
/// it's installed, or there's no version for this system).
pub fn install_command(fix: Fix) -> Option<String> {
    let tool = Tool::of(fix)?;
    let os = Os::current();
    match plan_for(os, tool, &Installers::here(tool)) {
        Plan::Ready => None,
        Plan::Run(steps) => Some(
            steps
                .iter()
                .map(|step| step.command_line_for(os))
                .collect::<Vec<_>>()
                .join(os.then()),
        ),
        Plan::Missing { command, .. } => (!command.is_empty()).then_some(command),
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
        Fix::InstallLightpanda => install(Tool::Lightpanda, progress),
        Fix::UseApiKey(backend) => super::providers::use_api_key(backend, progress),
        Fix::SignIn(backend) => {
            sign_in(backend)?;
            let window = if cfg!(target_os = "macos") {
                "Terminal window"
            } else {
                "terminal window"
            };
            progress(&format!(
                "Sign in to {} in the {window} Hex opened…",
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
    crate::desktop::opencode_catalog::open_url(url)
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

/// A terminal window running the agent's sign-in: on macOS a `.command`
/// file Terminal opens, on Linux the desktop's terminal running a script,
/// on Windows a console window running a batch file.
fn sign_in(backend: AssistantBackend) -> Result<()> {
    let path =
        super::executable(backend).ok_or_else(|| eyre!("{} isn't installed", name(backend)))?;
    let arguments = sign_in_arguments(backend)
        .ok_or_else(|| eyre!("The Claude API needs a key, not a sign-in"))?;
    let folder = std::env::temp_dir().join("hex-assistant");
    std::fs::create_dir_all(&folder)?;
    open_sign_in(&folder, &path, arguments, name(backend))
}

#[cfg(target_os = "macos")]
fn open_sign_in(folder: &Path, path: &Path, arguments: &str, agent: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let script = folder.join(format!("Sign in to {agent}.command"));
    let quoted = quote(&path.to_string_lossy(), Os::Mac);
    std::fs::write(
        &script,
        format!("#!/bin/zsh -l\nclear\nexec {quoted}{arguments}\n"),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let status = crate::desktop::background_process::command("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(&script)
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| eyre!("Couldn't open Terminal"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_sign_in(folder: &Path, path: &Path, arguments: &str, agent: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let script = folder.join(format!(
        "sign-in-{}.sh",
        agent.to_lowercase().replace(' ', "-")
    ));
    let quoted = quote(&path.to_string_lossy(), Os::Linux);
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nclear\nprintf 'Sign in to {agent}\\n\\n'\n{quoted}{arguments}\n\
             printf '\\nYou can close this window.\\n'\nread -r _ || true\n"
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    let script = script.to_string_lossy().into_owned();
    for (terminal, before) in linux_terminals() {
        let Some(program) = search::find(terminal) else {
            continue;
        };
        let mut command = crate::desktop::background_process::command(program);
        command
            .args(*before)
            .arg(&script)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match command.spawn() {
            Ok(mut child) => {
                // Some terminals return at once, others when the window
                // closes; either way it is reaped.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            Err(error) => tracing::debug!(terminal, %error, "couldn't open this terminal"),
        }
    }
    Err(eyre!(
        "Hex couldn't find a terminal to open. Run `{quoted}{arguments}` in one yourself"
    ))
}

/// Terminals to try, each with what goes before the command it runs: the
/// desktop's own choice first (`xdg-terminal-exec`, Debian's
/// `x-terminal-emulator`), then the common ones.
#[cfg(all(unix, not(target_os = "macos")))]
fn linux_terminals() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("xdg-terminal-exec", &[]),
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("kgx", &["--"]),
        ("ptyxis", &["--"]),
        ("konsole", &["-e"]),
        ("xfce4-terminal", &["-x"]),
        ("mate-terminal", &["-x"]),
        ("tilix", &["-e"]),
        ("kitty", &[]),
        ("alacritty", &["-e"]),
        ("wezterm", &["start", "--"]),
        ("foot", &[]),
        ("xterm", &["-e"]),
    ]
}

#[cfg(windows)]
fn open_sign_in(folder: &Path, path: &Path, arguments: &str, agent: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let script = folder.join(format!(
        "sign-in-{}.cmd",
        agent.to_lowercase().replace(' ', "-")
    ));
    std::fs::write(
        &script,
        format!(
            "@echo off\r\ntitle Sign in to {agent}\r\ncls\r\ncall \"{}\"{arguments}\r\n\
             echo.\r\necho You can close this window.\r\n",
            path.display()
        ),
    )?;
    let shell = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
    // A console window of its own (not `background_process`, which has
    // none); `/k` keeps it open. The doubled quotes survive cmd's rule of
    // stripping the outer pair.
    std::process::Command::new(shell)
        .raw_arg(format!("/d /k \"\"{}\"\"", script.display()))
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|mut child| {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        })
        .map_err(|error| eyre!("Couldn't open a console window: {error}"))
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
                "{} isn't signed in yet. Finish in the terminal window, then try again",
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
            ..Installers::default()
        }
    }

    /// The macOS plan (the tests below without an `os` are macOS's).
    fn plan(tool: Tool, here: &Installers) -> Plan {
        plan_for(Os::Mac, tool, here)
    }

    fn lines(plan: Plan) -> Vec<String> {
        lines_on(Os::Mac, plan)
    }

    fn lines_on(os: Os, plan: Plan) -> Vec<String> {
        match plan {
            Plan::Run(steps) => steps.iter().map(|step| step.command_line_for(os)).collect(),
            other => panic!("not a run: {other:?}"),
        }
    }

    fn linux(npm: Option<&str>, brew: bool) -> Installers {
        Installers {
            brew: brew.then(|| PathBuf::from("/home/linuxbrew/.linuxbrew/bin/brew")),
            npm: npm.map(PathBuf::from),
            npm_prefix: npm
                .filter(|npm| npm.starts_with("/usr/"))
                .map(|_| PathBuf::from("/home/me/.local")),
            ..Installers::default()
        }
    }

    fn windows(winget: bool, npm: bool) -> Installers {
        Installers {
            winget: winget.then(|| {
                PathBuf::from(r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\winget.exe")
            }),
            npm: npm.then(|| PathBuf::from(r"C:\Program Files\nodejs\npm.cmd")),
            powershell: Some(PathBuf::from(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            )),
            ..Installers::default()
        }
    }

    #[test]
    fn linux_uses_npm_into_the_home_folder_when_npm_is_the_systems() {
        let system = linux(Some("/usr/bin/npm"), false);
        assert_eq!(
            lines_on(Os::Linux, plan_for(Os::Linux, Tool::AgentDesktop, &system)),
            ["/usr/bin/npm install -g --prefix /home/me/.local --no-fund --no-audit agent-desktop"]
        );
        let own = linux(Some("/home/me/.nvm/versions/node/v22.1.0/bin/npm"), true);
        assert_eq!(
            lines_on(
                Os::Linux,
                plan_for(Os::Linux, Tool::Agent(AssistantBackend::Codex), &own)
            ),
            [
                "/home/me/.nvm/versions/node/v22.1.0/bin/npm install -g --no-fund --no-audit @openai/codex"
            ]
        );
        // Claude Code's native installer, even with npm there.
        assert_eq!(
            lines_on(
                Os::Linux,
                plan_for(Os::Linux, Tool::Agent(AssistantBackend::ClaudeCode), &own)
            ),
            ["bash -c 'curl -fsSL https://claude.ai/install.sh | bash'"]
        );
    }

    #[test]
    fn linux_without_npm_uses_scripts_then_linuxbrew_formulas() {
        let bare = linux(None, false);
        assert_eq!(
            lines_on(
                Os::Linux,
                plan_for(Os::Linux, Tool::Agent(AssistantBackend::OpenCode), &bare)
            ),
            ["bash -c 'curl -fsSL https://opencode.ai/v2/install | bash'"]
        );
        assert_eq!(
            lines_on(
                Os::Linux,
                plan_for(Os::Linux, Tool::AgentBrowser, &linux(None, true))
            ),
            [
                "/home/linuxbrew/.linuxbrew/bin/brew install agent-browser",
                "agent-browser install"
            ]
        );
        let Plan::Missing { why, command } = plan_for(Os::Linux, Tool::AgentDesktop, &bare) else {
            panic!("expected Missing");
        };
        assert!(why.starts_with("Node.js and npm aren't installed"), "{why}");
        assert!(
            !why.contains("Homebrew") && !why.contains("Terminal"),
            "{why}"
        );
        assert_eq!(
            command,
            "sudo apt install -y nodejs npm && npm install -g --prefix ~/.local agent-desktop"
        );
    }

    #[test]
    fn windows_uses_winget_then_npm_then_powershell() {
        let all = windows(true, true);
        assert_eq!(
            lines_on(
                Os::Windows,
                plan_for(Os::Windows, Tool::Agent(AssistantBackend::ClaudeCode), &all)
            ),
            [
                r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\winget.exe install --id Anthropic.ClaudeCode --exact --source winget --silent --disable-interactivity --accept-source-agreements --accept-package-agreements"
            ]
        );
        assert_eq!(
            lines_on(
                Os::Windows,
                plan_for(Os::Windows, Tool::Agent(AssistantBackend::Codex), &all)
            ),
            [
                r"& 'C:\Program Files\nodejs\npm.cmd' install -g --no-fund --no-audit '@openai/codex'"
            ]
        );
        assert_eq!(
            lines_on(Os::Windows, plan_for(Os::Windows, Tool::AgentBrowser, &all))[1],
            "agent-browser install"
        );
        // No WinGet: Claude Code's PowerShell script, not its deprecated npm.
        assert_eq!(
            lines_on(
                Os::Windows,
                plan_for(
                    Os::Windows,
                    Tool::Agent(AssistantBackend::ClaudeCode),
                    &windows(false, true)
                )
            ),
            [
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe -NoProfile -ExecutionPolicy Bypass -Command 'irm https://claude.ai/install.ps1 | iex'"
            ]
        );
        let Plan::Missing { why, command } =
            plan_for(Os::Windows, Tool::AgentDesktop, &windows(true, false))
        else {
            panic!("expected Missing");
        };
        assert!(why.starts_with("Node.js isn't installed"), "{why}");
        assert!(
            !why.contains("Homebrew") && !why.contains("Terminal"),
            "{why}"
        );
        assert!(
            command.starts_with("winget install --id OpenJS.NodeJS.LTS"),
            "{command}"
        );
        assert!(
            command.ends_with("; npm install -g agent-desktop"),
            "{command}"
        );
    }

    #[test]
    fn lightpanda_has_nothing_to_install_on_windows() {
        let Plan::Missing { why, command } =
            plan_for(Os::Windows, Tool::Lightpanda, &windows(true, true))
        else {
            panic!("expected Missing");
        };
        assert!(why.contains("no Windows version"), "{why}");
        assert!(command.is_empty());
        assert!(!Tool::Lightpanda.available_on(Os::Windows));
        assert!(Tool::Lightpanda.available_on(Os::Linux));
    }

    #[test]
    fn words_are_quoted_for_each_shell() {
        assert_eq!(quote("it's", Os::Mac), r"'it'\''s'");
        assert_eq!(quote("it's", Os::Windows), "'it''s'");
        assert_eq!(quote(r"C:\x\npm.cmd", Os::Windows), r"C:\x\npm.cmd");
        assert_eq!(quote("@openai/codex", Os::Windows), "'@openai/codex'");
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
    fn lightpanda_always_uses_its_own_installer() {
        for here in [here(true, true), here(false, false)] {
            let steps = lines(plan(Tool::Lightpanda, &here));
            assert_eq!(steps.len(), 1);
            assert!(
                steps[0].starts_with(
                    "/bin/bash -c 'curl -fsSL https://pkg.lightpanda.io/install.sh | bash -s"
                ),
                "{steps:?}"
            );
            assert!(steps[0].contains("releases/latest"));
        }
        assert_eq!(Tool::of(Fix::InstallLightpanda), Some(Tool::Lightpanda));
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
