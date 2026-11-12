use std::path::{Path, PathBuf};

use super::sign_in::{sign_in_arguments, terminal_args};
use super::*;

fn here(brew: bool, npm: bool) -> Installers {
    Installers {
        brew: brew.then(|| PathBuf::from("/opt/homebrew/bin/brew")),
        npm: npm.then(|| PathBuf::from("/Users/me/.fnm/bin/npm")),
        installed: false,
        browser_ready: false,
        jq: true,
    }
}

fn lines_on(os: Os, plan: Plan) -> Vec<String> {
    match plan {
        Plan::Run(steps) => steps.iter().map(|step| step.command_line(os)).collect(),
        other => panic!("not a run: {other:?}"),
    }
}

fn lines(plan: Plan) -> Vec<String> {
    lines_on(Os::Mac, plan)
}

fn mac(tool: Tool, here: &Installers) -> Plan {
    plan_for(Os::Mac, tool, here)
}

#[test]
fn homebrew_comes_first() {
    let brew = here(true, true);
    let claude = Tool::Agent(AssistantBackend::ClaudeCode);
    assert_eq!(
        lines(mac(claude, &brew)),
        ["/opt/homebrew/bin/brew install --cask claude-code"]
    );
    assert_eq!(
        lines(mac(Tool::Agent(AssistantBackend::Codex), &brew)),
        ["/opt/homebrew/bin/brew install --cask codex"]
    );
    assert_eq!(
        lines(mac(Tool::Agent(AssistantBackend::OpenCode), &brew)),
        ["/opt/homebrew/bin/brew install anomalyco/tap/opencode-v2"]
    );
    assert_eq!(
        lines(mac(Tool::AgentDesktop, &brew)),
        ["/opt/homebrew/bin/brew install agent-desktop"]
    );
    assert_eq!(
        lines(mac(Tool::AgentBrowser, &brew)),
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
        lines(mac(Tool::Agent(AssistantBackend::ClaudeCode), &npm)),
        ["/Users/me/.fnm/bin/npm install -g --no-fund --no-audit @anthropic-ai/claude-code"]
    );
    assert_eq!(
        lines(mac(Tool::Agent(AssistantBackend::OpenCode), &npm)),
        ["/Users/me/.fnm/bin/npm install -g --no-fund --no-audit @opencode/cli"]
    );
    assert_eq!(
        lines(mac(Tool::AgentBrowser, &npm))[1],
        "agent-browser install"
    );
}

#[test]
fn agents_fall_back_to_their_own_scripts() {
    let bare = here(false, false);
    assert_eq!(
        lines(mac(Tool::Agent(AssistantBackend::ClaudeCode), &bare)),
        ["/bin/bash -c 'curl -fsSL https://claude.ai/install.sh | bash'"]
    );
    assert_eq!(
        lines(mac(Tool::Agent(AssistantBackend::Codex), &bare)),
        ["/bin/bash -c 'curl -fsSL https://chatgpt.com/codex/install.sh | sh'"]
    );
    assert_eq!(
        lines(mac(Tool::Agent(AssistantBackend::OpenCode), &bare)),
        ["/bin/bash -c 'curl -fsSL https://opencode.ai/v2/install | bash'"]
    );
}

#[test]
fn without_homebrew_or_npm_the_tools_say_what_to_paste() {
    let Plan::Missing { why, command } = mac(Tool::AgentBrowser, &here(false, false)) else {
        panic!("expected Missing");
    };
    assert!(why.starts_with("Homebrew isn't installed"), "{why}");
    assert!(command.starts_with("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""));
    assert!(command.ends_with("&& brew install agent-browser && agent-browser install"));
}

#[test]
fn lightpanda_always_uses_its_own_installer() {
    for here in [here(true, true), here(false, false)] {
        let steps = lines(mac(Tool::Lightpanda, &here));
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
    for os in [Os::Mac, Os::Linux, Os::Windows] {
        let mut installed = here(true, true);
        installed.installed = true;
        assert_eq!(
            plan_for(os, Tool::Agent(AssistantBackend::Codex), &installed),
            Plan::Ready
        );
        assert_eq!(plan_for(os, Tool::AgentDesktop, &installed), Plan::Ready);
        // agent-browser without its Chrome only downloads Chrome.
        assert_eq!(
            lines_on(os, plan_for(os, Tool::AgentBrowser, &installed)),
            ["agent-browser install"]
        );
        installed.browser_ready = true;
        assert_eq!(plan_for(os, Tool::AgentBrowser, &installed), Plan::Ready);
    }
}

#[test]
fn linux_takes_npm_then_homebrew_formulae_then_the_scripts() {
    let both = here(true, true);
    let linux = |tool, here: &Installers| plan_for(Os::Linux, tool, here);
    assert_eq!(
        lines_on(Os::Linux, linux(Tool::AgentDesktop, &both)),
        ["/Users/me/.fnm/bin/npm install -g --no-fund --no-audit agent-desktop"]
    );
    // Homebrew on Linux, for the tools it has, without npm.
    let brew = here(true, false);
    assert_eq!(
        lines_on(Os::Linux, linux(Tool::AgentDesktop, &brew)),
        ["/opt/homebrew/bin/brew install agent-desktop"]
    );
    // No casks on Linux: Claude Code takes its own script, run by bash.
    assert_eq!(
        lines_on(
            Os::Linux,
            linux(Tool::Agent(AssistantBackend::ClaudeCode), &brew)
        ),
        ["bash -c 'curl -fsSL https://claude.ai/install.sh | bash'"]
    );
    let Plan::Missing { why, command } = linux(Tool::AgentBrowser, &here(false, false)) else {
        panic!("expected Missing");
    };
    assert!(why.starts_with("Node.js isn't installed"), "{why}");
    assert!(
        why.contains("terminal") && !why.contains("Homebrew"),
        "{why}"
    );
    assert_eq!(
        command,
        "npm install -g --no-fund --no-audit agent-browser && agent-browser install"
    );
    // Lightpanda's installer needs jq.
    let mut no_jq = here(false, false);
    no_jq.jq = false;
    let Plan::Missing { why, .. } = linux(Tool::Lightpanda, &no_jq) else {
        panic!("expected Missing");
    };
    assert!(why.contains("jq"), "{why}");
}

#[test]
fn windows_takes_npm_then_claude_codes_powershell_installer() {
    let npm = Installers {
        npm: Some(PathBuf::from(r"C:\Program Files\nodejs\npm.cmd")),
        ..here(false, false)
    };
    let windows = |tool, here: &Installers| plan_for(Os::Windows, tool, here);
    assert_eq!(
        lines_on(Os::Windows, windows(Tool::AgentDesktop, &npm)),
        [r"& 'C:\Program Files\nodejs\npm.cmd' install -g --no-fund --no-audit agent-desktop"]
    );
    let bare = here(false, false);
    let Plan::Run(steps) = windows(Tool::Agent(AssistantBackend::ClaudeCode), &bare) else {
        panic!("expected a run");
    };
    assert_eq!(steps[0].program, "powershell");
    assert_eq!(
        steps[0].args.last().map(String::as_str),
        Some("irm https://claude.ai/install.ps1 | iex")
    );
    // What a person pastes is the installer's own line.
    assert_eq!(
        steps[0].command_line(Os::Windows),
        "irm https://claude.ai/install.ps1 | iex"
    );
    // Homebrew is never used, and Codex without npm says what to paste.
    let brew = here(true, false);
    let Plan::Missing { why, command } = windows(Tool::Agent(AssistantBackend::Codex), &brew)
    else {
        panic!("expected Missing");
    };
    assert!(
        why.contains("nodejs.org") && why.contains("PowerShell"),
        "{why}"
    );
    assert_eq!(command, "npm install -g --no-fund --no-audit @openai/codex");
    let Plan::Missing { command, .. } = windows(Tool::AgentBrowser, &bare) else {
        panic!("expected Missing");
    };
    assert!(command.ends_with("; agent-browser install"), "{command}");
}

#[test]
fn lightpanda_has_no_windows_build() {
    for here in [here(true, true), here(false, false)] {
        assert_eq!(
            plan_for(Os::Windows, Tool::Lightpanda, &here),
            Plan::Unavailable(NO_LIGHTPANDA_ON_WINDOWS.into())
        );
    }
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
        Some(&["auth", "login"][..])
    );
    assert_eq!(sign_in_arguments(AssistantBackend::ClaudeApi), None);
}

#[test]
fn each_linux_terminal_is_told_to_run_the_sign_in_its_own_way() {
    let program = ["/home/f/.local/bin/claude", "auth", "login"].map(str::to_owned);
    assert_eq!(
        terminal_args(Path::new("/usr/bin/gnome-terminal"), &program),
        ["--", "/home/f/.local/bin/claude", "auth", "login"]
    );
    assert_eq!(
        terminal_args(Path::new("/usr/bin/kitty"), &program),
        program
    );
    assert_eq!(
        terminal_args(Path::new("/usr/bin/some-term"), &program)[0],
        "-e"
    );
}
