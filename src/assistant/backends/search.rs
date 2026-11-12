//! Where the agents' executables are, for a GUI app that never got the
//! person's shell PATH: the directories installers use, and the login
//! shell's own answer, asked once (bounded) and kept.
//!
//! fnm puts a per-shell "multishell" directory first on PATH that is gone
//! once that shell closes; it is a link to an alias (`aliases/default`), and
//! that alias is what Hex keeps.
//!
//! On Windows there is no login shell to ask: the PATH comes from the
//! registry (the machine's, then the person's, as a new terminal gets it),
//! names are looked up with their extensions (`.exe` first, never the
//! extensionless shell script npm also writes), and an npm `.cmd` shim is
//! run as the `node.exe <script>` (or the `.exe`) it wraps ([`launch`]):
//! Rust refuses, or would have `cmd.exe` mangle, arguments with newlines or
//! quotes for a batch file, and agents get just such arguments.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

/// The executables the login shell is asked for.
pub const NAMES: [&str; 6] = [
    "claude",
    "codex",
    "opencode",
    "agent-desktop",
    "node",
    "brew",
];

/// How long the login shell may take to answer.
const SHELL_LIMIT: Duration = Duration::from_secs(6);

const PATH_MARK: &str = "__HEX_PATH__=";
const BIN_MARK: &str = "__HEX_BIN__";

/// What the login shell said.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Shell {
    /// Its PATH, in order.
    pub path: Vec<PathBuf>,
    /// `command -v` for each of [`NAMES`] it found.
    pub found: Vec<(String, PathBuf)>,
}

static SHELL: Mutex<Option<Shell>> = Mutex::new(None);

/// The login shell's answer, asked on the first call (up to a few seconds)
/// and kept until [`forget`].
pub fn shell() -> Shell {
    let mut cached = SHELL.lock().unwrap_or_else(|error| error.into_inner());
    cached.get_or_insert_with(ask_shell).clone()
}

/// Forgets the login shell's answer (after an install).
pub fn forget() {
    *SHELL.lock().unwrap_or_else(|error| error.into_inner()) = None;
}

/// The person's login shell: `$SHELL`, else their account's, else the
/// system's usual one (zsh on macOS, else bash, else sh).
#[cfg(unix)]
fn login_shell() -> PathBuf {
    let usable = |shell: &PathBuf| shell.is_absolute() && shell.exists();
    let fallback = if cfg!(target_os = "macos") {
        &["/bin/zsh", "/bin/bash", "/bin/sh"][..]
    } else {
        &["/bin/bash", "/usr/bin/bash", "/bin/sh"][..]
    };
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(usable)
        .or_else(|| account_shell().filter(usable))
        .or_else(|| fallback.iter().map(PathBuf::from).find(usable))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

#[cfg(unix)]
fn account_shell() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: getpwuid returns a pointer into static storage or null; the
    // shell is copied out at once.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if entry.is_null() || (*entry).pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr((*entry).pw_shell);
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(shell.to_bytes())))
    }
}

/// Windows: the PATH a new terminal would get (machine, then user, from
/// the registry), then Hex's own. Names are found in [`dirs`].
#[cfg(windows)]
fn ask_shell() -> Shell {
    let mut path = registry_path();
    if let Some(own) = std::env::var_os("PATH") {
        path.extend(std::env::split_paths(&own));
    }
    path.retain(|dir| dir.is_absolute());
    Shell {
        path,
        found: Vec::new(),
    }
}

#[cfg(windows)]
fn registry_path() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let read = |root, key: &str| -> Option<String> {
        RegKey::predef(root)
            .open_subkey(key)
            .ok()?
            .get_value::<String, _>("Path")
            .ok()
    };
    let lookup = |name: &str| std::env::var(name).ok();
    [
        read(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
        read(HKEY_CURRENT_USER, "Environment"),
    ]
    .into_iter()
    .flatten()
    .flat_map(|value| {
        expand(&value, lookup)
            .split(';')
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    })
    .collect()
}

/// `%NAME%` in a registry value replaced by `lookup(NAME)`; unknown names
/// stay as written.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn expand(value: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(found) => out.push_str(&found),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(unix)]
fn ask_shell() -> Shell {
    let shell = login_shell();
    let names = NAMES.join(" ");
    let script = format!(
        "printf '\\n{PATH_MARK}%s\\n' \"$PATH\"; for n in {names}; do \
         p=$(command -v \"$n\" 2>/dev/null); [ -n \"$p\" ] && printf '{BIN_MARK}%s=%s\\n' \"$n\" \"$p\"; \
         done; exit 0"
    );
    let mut command = crate::desktop::background_process::command(&shell);
    // Interactive too: many people set PATH in .zshrc, not .zprofile.
    command.args(["-l", "-i", "-c", &script]);
    match super::run::bounded(command, SHELL_LIMIT) {
        Ok(ran) => {
            if ran.timed_out() {
                tracing::warn!(shell = %shell.display(), "the login shell took too long to say its PATH");
            }
            parse_shell(&ran.stdout)
        }
        Err(error) => {
            tracing::warn!(%error, shell = %shell.display(), "could not ask the login shell for its PATH");
            Shell::default()
        }
    }
}

/// The marked lines a login shell printed, between whatever its startup
/// files print.
pub fn parse_shell(output: &str) -> Shell {
    let mut shell = Shell::default();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(path) = line.split_once(PATH_MARK).map(|(_, path)| path) {
            shell.path = std::env::split_paths(path)
                .filter(|dir| dir.is_absolute())
                .collect();
        } else if let Some((name, path)) = line
            .split_once(BIN_MARK)
            .and_then(|(_, found)| found.split_once('='))
        {
            let path = PathBuf::from(path.trim());
            // `command -v` names aliases and functions without a slash.
            if path.is_absolute() && NAMES.contains(&name) {
                shell.found.push((name.to_owned(), path));
            }
        }
    }
    shell
}

/// `path` with an fnm multishell directory replaced by what it links to (an
/// alias such as `aliases/default`), so it outlives the shell that made it.
pub fn stable(path: &Path, read_link: impl Fn(&Path) -> Option<PathBuf>) -> PathBuf {
    let mut prefix = PathBuf::new();
    let mut components = path.components();
    while let Some(component) = components.next() {
        prefix.push(component);
        if component.as_os_str() == "fnm_multishells"
            && let Some(shell) = components.next()
        {
            prefix.push(shell);
            let Some(target) = read_link(&prefix) else {
                return path.to_owned();
            };
            let target = if target.is_absolute() {
                target
            } else {
                prefix.parent().unwrap_or(Path::new("/")).join(target)
            };
            return target.join(components.as_path());
        }
    }
    path.to_owned()
}

fn read_link(path: &Path) -> Option<PathBuf> {
    std::fs::read_link(path).ok()
}

/// Where installers put these tools, most specific first; directories that
/// don't exist are left out by the caller.
pub fn known_dirs(home: &Path, fnm_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".opencode/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
    ];
    // fnm's default alias, wherever its data lives.
    let fnm_roots = [
        fnm_dir.map(Path::to_path_buf),
        Some(home.join(".local/share/fnm")),
        Some(home.join("Library/Application Support/fnm")),
        Some(home.join(".fnm")),
    ];
    dirs.extend(
        fnm_roots
            .into_iter()
            .flatten()
            .map(|root| root.join("aliases/default/bin")),
    );
    dirs.extend([
        home.join(".nvm/current/bin"),
        home.join(".asdf/shims"),
        home.join(".local/share/mise/shims"),
        home.join(".cargo/bin"),
        home.join(".deno/bin"),
        PathBuf::from("/opt/local/bin"),
        // Linux: Linuxbrew, pnpm, snaps.
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        home.join(".linuxbrew/bin"),
        home.join(".local/share/pnpm"),
        home.join("Library/pnpm"),
        PathBuf::from("/snap/bin"),
    ]);
    dirs
}

/// Windows: where installers put these tools, from the environment's
/// folders (`lookup`): npm's global folder, WinGet's links, the native
/// Claude Code installer's `.local\bin`, Bun, Scoop, pnpm, Volta, fnm's and
/// nvm-windows' current Node, Node's own installer, then System32.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_known_dirs(lookup: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let var = |name: &str| {
        lookup(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let profile = var("USERPROFILE");
    let appdata = var("APPDATA");
    let local = var("LOCALAPPDATA");
    let program_files = var("ProgramFiles");
    let system_root = var("SystemRoot").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let under = |base: &Option<PathBuf>, rest: &str| base.as_ref().map(|base| base.join(rest));
    let fnm_dir = var("FNM_DIR").or_else(|| under(&appdata, "fnm"));
    [
        under(&appdata, "npm"),
        under(&local, r"Microsoft\WinGet\Links"),
        under(&program_files, r"WinGet\Links"),
        under(&profile, r".local\bin"),
        under(&profile, r".bun\bin"),
        under(&profile, r".opencode\bin"),
        under(&profile, r"scoop\shims"),
        under(&local, "pnpm"),
        under(&local, r"Volta\bin"),
        fnm_dir.map(|dir| dir.join(r"aliases\default")),
        var("NVM_SYMLINK"),
        under(&program_files, "nodejs"),
        under(&profile, r".cargo\bin"),
        Some(system_root.join("System32")),
        Some(system_root.clone()),
        Some(system_root.join(r"System32\WindowsPowerShell\v1.0")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// nvm's installed Node versions' `bin`, newest first.
fn nvm_dirs(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) else {
        return Vec::new();
    };
    let mut versions: Vec<(Vec<u64>, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            Some((version_key(&name)?, entry.path().join("bin")))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().map(|(_, dir)| dir).collect()
}

/// `v22.3.1` → `[22, 3, 1]`.
fn version_key(name: &str) -> Option<Vec<u64>> {
    name.trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().ok())
        .collect()
}

fn home() -> Option<PathBuf> {
    crate::app_paths::home_dir()
}

/// Every directory to look in, in order: the login shell's PATH (made
/// stable), then the known install directories, then the system's. Only
/// directories that exist, each once.
pub fn dirs() -> Vec<PathBuf> {
    let shell = shell();
    let mut out: Vec<PathBuf> = Vec::new();
    let all = shell
        .path
        .iter()
        .map(|dir| stable(dir, read_link))
        .chain(install_dirs());
    for dir in all {
        if !out.contains(&dir) && dir.is_dir() {
            out.push(dir);
        }
    }
    out
}

#[cfg(unix)]
fn install_dirs() -> Vec<PathBuf> {
    let home = home().unwrap_or_else(|| PathBuf::from("/"));
    let fnm_dir = std::env::var_os("FNM_DIR").map(PathBuf::from);
    let mut dirs = known_dirs(&home, fnm_dir.as_deref());
    dirs.extend(nvm_dirs(&home));
    dirs.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    dirs
}

#[cfg(windows)]
fn install_dirs() -> Vec<PathBuf> {
    windows_known_dirs(|name| std::env::var(name).ok())
}

/// The executable `name`: where the login shell finds it (made stable),
/// else the first of [`dirs`] that has it. On Windows, `name.exe` (or
/// `.com`) anywhere on [`dirs`] comes before an npm `name.cmd` shim.
#[cfg(windows)]
pub fn find(name: &str) -> Option<PathBuf> {
    let dirs = dirs();
    if Path::new(name).extension().is_some() {
        return dirs
            .iter()
            .map(|dir| dir.join(name))
            .find(|path| is_executable(path));
    }
    windows_extensions().iter().find_map(|group| {
        dirs.iter().find_map(|dir| {
            group
                .iter()
                .map(|extension| dir.join(format!("{name}{extension}")))
                .find(|path| is_executable(path))
        })
    })
}

/// The extensions tried, in groups: programs, then batch files. From
/// PATHEXT, less scripts Hex can't start as a program.
#[cfg(windows)]
fn windows_extensions() -> [Vec<String>; 2] {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let listed: Vec<String> = pathext
        .split(';')
        .map(|extension| extension.trim().to_ascii_lowercase())
        .collect();
    let pick = |wanted: &[&str]| {
        wanted
            .iter()
            .filter(|extension| listed.iter().any(|listed| listed == *extension))
            .map(|extension| (*extension).to_owned())
            .collect::<Vec<_>>()
    };
    let mut programs = pick(&[".exe", ".com"]);
    if programs.is_empty() {
        programs.push(".exe".into());
    }
    let mut scripts = pick(&[".cmd", ".bat"]);
    if scripts.is_empty() {
        scripts.push(".cmd".into());
    }
    [programs, scripts]
}

/// The executable `name`: where the login shell finds it (made stable),
/// else the first of [`dirs`] that has it.
#[cfg(unix)]
pub fn find(name: &str) -> Option<PathBuf> {
    let shell = shell();
    shell
        .found
        .iter()
        .filter(|(found, _)| found == name)
        .map(|(_, path)| stable(path, read_link))
        .find(|path| is_executable(path))
        .or_else(|| {
            dirs()
                .into_iter()
                .map(|dir| dir.join(name))
                .find(|path| is_executable(path))
        })
}

pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// A `PATH` value for an agent Hex starts: [`dirs`], plus the directories
/// of `extra` executables (an agent found outside them).
pub fn path_value(extra: &[PathBuf]) -> String {
    let mut dirs = dirs();
    for dir in extra.iter().filter_map(|path| path.parent()) {
        if !dirs.iter().any(|known| known == dir) {
            dirs.push(dir.to_owned());
        }
    }
    std::env::join_paths(dirs)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                std::env::var("PATH").unwrap_or_default()
            } else {
                "/usr/bin:/bin:/usr/sbin:/sbin".into()
            }
        })
}

/// What actually runs `path`: the program and the arguments that go before
/// the caller's. Itself, except a Windows npm (or pnpm) `.cmd` shim, which
/// becomes the `.exe` it wraps or `node.exe <its script>`.
pub fn launch(path: &Path) -> (PathBuf, Vec<OsString>) {
    #[cfg(windows)]
    if let Some(launch) = unshim(path) {
        return launch;
    }
    (path.to_owned(), Vec::new())
}

/// A command that runs `path` ([`launch`]) and never shows a window.
pub fn command(path: &Path) -> Command {
    let (program, lead) = launch(path);
    let mut command = crate::desktop::background_process::command(program);
    command.args(lead);
    command
}

#[cfg(windows)]
fn unshim(path: &Path) -> Option<(PathBuf, Vec<OsString>)> {
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if extension != "cmd" && extension != "bat" {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 64 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let target = path.parent()?.join(shim_target(&text)?);
    if !target.is_file() {
        return None;
    }
    let is_exe = target
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if is_exe {
        return Some((target, Vec::new()));
    }
    let beside = path.parent()?.join("node.exe");
    let node = if beside.is_file() {
        beside
    } else {
        find("node").filter(|node| {
            node.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        })?
    };
    Some((node, vec![target.into_os_string()]))
}

/// The script (or program) a `cmd-shim` batch file runs, relative to the
/// shim's folder: the last `"%dp0%\…"` (or `"%~dp0\…"`) path in it that
/// isn't `node.exe`. `None` for a batch file that isn't a shim.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn shim_target(text: &str) -> Option<String> {
    text.split('"')
        .filter_map(|piece| {
            let lower = piece.to_ascii_lowercase();
            ["%dp0%", "%~dp0"]
                .iter()
                .find(|prefix| lower.starts_with(*prefix))
                .map(|prefix| piece[prefix.len()..].trim_start_matches(['\\', '/']))
        })
        .filter(|rest| {
            !rest.is_empty()
                && !rest.eq_ignore_ascii_case("node.exe")
                && !rest.eq_ignore_ascii_case("node")
                && !rest.contains('%')
        })
        .last()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shells_marked_lines_are_read_between_its_noise() {
        let output = "Welcome back!\n\
            \n__HEX_PATH__=/Users/f/.local/state/fnm_multishells/1067_17941/bin:/opt/homebrew/bin:relative:/usr/bin\n\
            __HEX_BIN__claude=/Users/f/.local/bin/claude\n\
            __HEX_BIN__codex=/Users/f/.local/state/fnm_multishells/1067_17941/bin/codex\n\
            __HEX_BIN__node=node: aliased to bun\n\
            __HEX_BIN__rm=/bin/rm\n\
            [oh-my-zsh] done\n";
        let shell = parse_shell(output);
        assert_eq!(
            shell.path,
            [
                "/Users/f/.local/state/fnm_multishells/1067_17941/bin",
                "/opt/homebrew/bin",
                "/usr/bin"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            shell.found,
            vec![
                (
                    "claude".to_owned(),
                    PathBuf::from("/Users/f/.local/bin/claude")
                ),
                (
                    "codex".to_owned(),
                    PathBuf::from("/Users/f/.local/state/fnm_multishells/1067_17941/bin/codex")
                ),
            ]
        );
    }

    #[test]
    fn a_silent_or_broken_shell_says_nothing() {
        assert_eq!(parse_shell(""), Shell::default());
        assert_eq!(
            parse_shell("zsh: command not found: compinit"),
            Shell::default()
        );
    }

    #[test]
    fn an_fnm_multishell_path_becomes_the_alias_it_links_to() {
        let link = |path: &Path| {
            (path == Path::new("/Users/f/.local/state/fnm_multishells/1067_17941"))
                .then(|| PathBuf::from("/Users/f/.local/share/fnm/aliases/default"))
        };
        assert_eq!(
            stable(
                Path::new("/Users/f/.local/state/fnm_multishells/1067_17941/bin/codex"),
                link
            ),
            PathBuf::from("/Users/f/.local/share/fnm/aliases/default/bin/codex")
        );
        assert_eq!(
            stable(
                Path::new("/Users/f/.local/state/fnm_multishells/1067_17941/bin"),
                link
            ),
            PathBuf::from("/Users/f/.local/share/fnm/aliases/default/bin")
        );
    }

    #[test]
    fn a_relative_multishell_link_resolves_beside_it() {
        let link = |_: &Path| Some(PathBuf::from("../share/fnm/aliases/default"));
        assert_eq!(
            stable(
                Path::new("/h/.local/state/fnm_multishells/9_1/bin/node"),
                link
            ),
            PathBuf::from("/h/.local/state/fnm_multishells/../share/fnm/aliases/default/bin/node")
        );
    }

    #[test]
    fn other_paths_and_a_vanished_multishell_stay_as_they_are() {
        let none = |_: &Path| None;
        for path in [
            "/opt/homebrew/bin/opencode",
            "/Users/f/.local/state/fnm_multishells/1067_17941/bin/codex",
        ] {
            assert_eq!(stable(Path::new(path), none), PathBuf::from(path));
        }
    }

    #[test]
    fn known_dirs_cover_the_installers_and_fnms_default_alias() {
        let dirs = known_dirs(Path::new("/Users/f"), Some(Path::new("/custom/fnm")));
        for dir in [
            "/Users/f/.local/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/Users/f/.npm-global/bin",
            "/Users/f/.bun/bin",
            "/Users/f/.volta/bin",
            "/Users/f/.opencode/bin",
            "/custom/fnm/aliases/default/bin",
            "/Users/f/.local/share/fnm/aliases/default/bin",
            "/Users/f/Library/Application Support/fnm/aliases/default/bin",
        ] {
            assert!(dirs.contains(&PathBuf::from(dir)), "{dir} is missing");
        }
    }

    #[test]
    fn known_dirs_cover_linuxbrew_pnpm_and_snaps() {
        let dirs = known_dirs(Path::new("/home/f"), None);
        for dir in [
            "/home/linuxbrew/.linuxbrew/bin",
            "/home/f/.local/share/pnpm",
            "/snap/bin",
            "/home/f/.npm-global/bin",
        ] {
            assert!(dirs.contains(&PathBuf::from(dir)), "{dir} is missing");
        }
    }

    #[test]
    fn windows_known_dirs_come_from_the_environment() {
        let env = |name: &str| {
            Some(
                match name {
                    "USERPROFILE" => r"C:\Users\f",
                    "APPDATA" => r"C:\Users\f\AppData\Roaming",
                    "LOCALAPPDATA" => r"C:\Users\f\AppData\Local",
                    "ProgramFiles" => r"C:\Program Files",
                    "SystemRoot" => r"C:\Windows",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let dirs: Vec<String> = windows_known_dirs(env)
            .iter()
            .map(|dir| dir.to_string_lossy().replace('/', "\\"))
            .collect();
        for dir in [
            r"C:\Users\f\AppData\Roaming\npm",
            r"C:\Users\f\AppData\Local\Microsoft\WinGet\Links",
            r"C:\Users\f\.local\bin",
            r"C:\Users\f\.bun\bin",
            r"C:\Users\f\AppData\Roaming\fnm\aliases\default",
            r"C:\Program Files\nodejs",
            r"C:\Windows\System32",
        ] {
            assert!(dirs.iter().any(|known| known == dir), "{dir} is missing");
        }
    }

    #[test]
    fn registry_values_expand_known_names_only() {
        let lookup = |name: &str| (name == "USERPROFILE").then(|| r"C:\Users\f".to_owned());
        assert_eq!(
            expand(r"%USERPROFILE%\.bun\bin;%NOPE%\x;50%", lookup),
            r"C:\Users\f\.bun\bin;%NOPE%\x;50%"
        );
    }

    #[test]
    fn an_npm_shim_names_the_script_it_runs() {
        let npm = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\n\
            SETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  \
            SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  \
            SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || \
            title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n";
        assert_eq!(
            shim_target(npm).as_deref(),
            Some(r"node_modules\@openai\codex\bin\codex.js")
        );
        let exe =
            "@ECHO off\r\n\"%~dp0\\node_modules\\agent-desktop\\bin\\agent-desktop.exe\"   %*\r\n";
        assert_eq!(
            shim_target(exe).as_deref(),
            Some(r"node_modules\agent-desktop\bin\agent-desktop.exe")
        );
        assert_eq!(shim_target("@echo off\r\necho hi\r\n"), None);
    }

    #[test]
    fn a_program_runs_as_itself_off_windows() {
        let (program, lead) = launch(Path::new("/usr/local/bin/codex"));
        assert_eq!(program, PathBuf::from("/usr/local/bin/codex"));
        if cfg!(unix) {
            assert!(lead.is_empty());
        }
    }

    #[test]
    fn node_versions_sort_newest_first() {
        assert!(version_key("v22.23.1") > version_key("v22.9.0"));
        assert!(version_key("v24.0.0") > version_key("v22.23.1"));
        assert_eq!(version_key("system"), None);
    }
}
