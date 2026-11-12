//! Signing in to an agent in a terminal window Hex opens (Terminal on
//! macOS, a console window on Windows, the person's terminal emulator on
//! Linux), and waiting until the agent says it's signed in.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use color_eyre::{Result, eyre::eyre};

use super::super::os::Os;
use super::super::{name, search};
use crate::app_settings::AssistantBackend;

/// How long Hex waits for a sign-in.
const SIGN_IN_LIMIT: Duration = Duration::from_secs(600);

/// The sign-in command for `backend`, after its executable.
pub(super) fn sign_in_arguments(backend: AssistantBackend) -> Option<&'static [&'static str]> {
    match backend {
        AssistantBackend::ClaudeCode => Some(&["auth", "login"]),
        AssistantBackend::Codex => Some(&["login"]),
        AssistantBackend::OpenCode => Some(&["auth", "login"]),
        AssistantBackend::ClaudeApi => None,
    }
}

/// What the window the sign-in runs in is called, for "Sign in … in the
/// {} Hex opened".
pub(super) fn window_name(os: Os) -> &'static str {
    match os {
        Os::Mac => "Terminal window",
        Os::Windows => "window",
        Os::Linux => "terminal window",
    }
}

/// Opens a terminal window running `backend`'s sign-in. Returns once it
/// is open.
pub(super) fn open(backend: AssistantBackend) -> Result<()> {
    let path = super::super::executable(backend)
        .ok_or_else(|| eyre!("{} isn't installed", name(backend)))?;
    let arguments = sign_in_arguments(backend)
        .ok_or_else(|| eyre!("The Claude API needs a key, not a sign-in"))?;
    match Os::HERE {
        Os::Mac => mac(backend, &path, arguments),
        Os::Windows => windows(&path, arguments),
        Os::Linux => linux(&path, arguments),
    }
}

/// Terminal, running the sign-in: a `.command` file Terminal opens.
fn mac(backend: AssistantBackend, path: &Path, arguments: &[&str]) -> Result<()> {
    let folder = std::env::temp_dir().join("hex-assistant");
    std::fs::create_dir_all(&folder)?;
    let script = folder.join(format!("Sign in to {}.command", name(backend)));
    let quoted = format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/zsh -l\nclear\nexec {quoted} {}\n",
            arguments.join(" ")
        ),
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

/// The agent in a console window of its own.
fn windows(path: &Path, arguments: &[&str]) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        let launch = super::super::launch::here(path);
        std::process::Command::new(&launch.program)
            .args(&launch.args)
            .args(arguments)
            .env_clear()
            .envs(super::super::agent_env())
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map_err(|error| eyre!("Couldn't open a window for the sign-in: {error}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (path, arguments);
        Err(eyre!("not Windows"))
    }
}

/// The person's terminal emulator, running the agent.
fn linux(path: &Path, arguments: &[&str]) -> Result<()> {
    let preferred = std::env::var_os("TERMINAL").map(PathBuf::from);
    let terminal = preferred
        .into_iter()
        .chain(TERMINALS.iter().map(|(name, _)| PathBuf::from(name)))
        .find_map(|terminal| {
            if terminal.is_absolute() {
                search::is_executable(&terminal).then_some(terminal)
            } else {
                search::find(&terminal.to_string_lossy())
            }
        })
        .ok_or_else(|| {
            eyre!(
                "Hex found no terminal app. Run `{} {}` in a terminal to sign in",
                path.display(),
                arguments.join(" ")
            )
        })?;
    let mut program = vec![path.to_string_lossy().into_owned()];
    program.extend(arguments.iter().map(|arg| (*arg).to_owned()));
    let args = terminal_args(&terminal, &program);
    super::super::agent_command(&terminal)
        .args(&args)
        .spawn()
        .map_err(|error| eyre!("Couldn't open {}: {error}", terminal.display()))?;
    Ok(())
}

/// Terminal emulators Hex tries on Linux, and how each is told to run a
/// command (its arguments go after).
const TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("ptyxis", &["--"]),
    ("gnome-terminal", &["--"]),
    ("kgx", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("foot", &[]),
    ("xterm", &["-e"]),
];

/// `terminal`'s arguments for running `program` (an executable and its
/// arguments); an unknown terminal gets `-e`, which most take.
pub(super) fn terminal_args(terminal: &Path, program: &[String]) -> Vec<String> {
    let name = terminal
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let flags: &[&str] = TERMINALS
        .iter()
        .find(|(known, _)| *known == name)
        .map_or(&["-e"], |(_, flags)| flags);
    flags
        .iter()
        .map(|flag| (*flag).to_owned())
        .chain(program.iter().cloned())
        .collect()
}

/// Asks the agent every few seconds whether it's signed in, for ten
/// minutes at most.
pub(super) fn wait(backend: AssistantBackend, progress: &mut dyn FnMut(&str)) -> Result<()> {
    let deadline = Instant::now() + SIGN_IN_LIMIT;
    let mut said_waiting = false;
    loop {
        std::thread::sleep(Duration::from_secs(3));
        let found = super::super::find(backend);
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
                "{} isn't signed in yet. Finish in the {} Hex opened, then try again",
                name(backend),
                window_name(Os::HERE)
            ));
        }
    }
}
