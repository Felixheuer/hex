//! Signing an agent in: its own sign-in command in a terminal window (a
//! `.command` file Terminal opens on macOS, the desktop's terminal on
//! Linux, a console window on Windows), while Agent-Desk asks the agent every few
//! seconds whether it's signed in.

use super::*;

/// How long Agent-Desk waits for a sign-in in Terminal.
const SIGN_IN_LIMIT: Duration = Duration::from_secs(600);

/// The sign-in command for `backend`, after its executable.
pub(super) fn sign_in_arguments(backend: AssistantBackend) -> Option<&'static str> {
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
pub(super) fn sign_in(backend: AssistantBackend) -> Result<()> {
    let path = super::super::executable(backend)
        .ok_or_else(|| eyre!("{} isn't installed", name(backend)))?;
    let arguments = sign_in_arguments(backend)
        .ok_or_else(|| eyre!("The Claude API needs a key, not a sign-in"))?;
    let folder = std::env::temp_dir().join("hex-assistant");
    std::fs::create_dir_all(&folder)?;
    open_sign_in(&folder, &path, arguments, name(backend))
}

#[cfg(target_os = "macos")]
pub(super) fn open_sign_in(folder: &Path, path: &Path, arguments: &str, agent: &str) -> Result<()> {
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
pub(super) fn open_sign_in(folder: &Path, path: &Path, arguments: &str, agent: &str) -> Result<()> {
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
        "Agent-Desk couldn't find a terminal to open. Run `{quoted}{arguments}` in one yourself"
    ))
}

/// Terminals to try, each with what goes before the command it runs: the
/// desktop's own choice first (`xdg-terminal-exec`, Debian's
/// `x-terminal-emulator`), then the common ones.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn linux_terminals() -> &'static [(&'static str, &'static [&'static str])] {
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
pub(super) fn open_sign_in(folder: &Path, path: &Path, arguments: &str, agent: &str) -> Result<()> {
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
pub(super) fn wait_for_sign_in(
    backend: AssistantBackend,
    progress: &mut dyn FnMut(&str),
) -> Result<()> {
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
                "{} isn't signed in yet. Finish in the terminal window, then try again",
                name(backend)
            ));
        }
    }
}
