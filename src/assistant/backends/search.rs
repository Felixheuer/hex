//! Where the agents' executables are, for a GUI app that never got the
//! person's shell PATH: the directories installers use, and the login
//! shell's own answer, asked once (bounded) and kept.
//!
//! fnm puts a per-shell "multishell" directory first on PATH that is gone
//! once that shell closes; it is a link to an alias (`aliases/default`), and
//! that alias is what Hex keeps.

use std::path::{Path, PathBuf};
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

/// The person's login shell: `$SHELL`, else their account's, else zsh.
fn login_shell() -> PathBuf {
    let usable = |shell: &PathBuf| shell.is_absolute() && shell.exists();
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(usable)
        .or_else(|| account_shell().filter(usable))
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"))
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

#[cfg(not(unix))]
fn account_shell() -> Option<PathBuf> {
    None
}

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
    ]);
    dirs
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
    let home = home().unwrap_or_else(|| PathBuf::from("/"));
    let fnm_dir = std::env::var_os("FNM_DIR").map(PathBuf::from);
    let system = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from);
    let mut out: Vec<PathBuf> = Vec::new();
    let all = shell
        .path
        .iter()
        .map(|dir| stable(dir, read_link))
        .chain(known_dirs(&home, fnm_dir.as_deref()))
        .chain(nvm_dirs(&home))
        .chain(system);
    for dir in all {
        if !out.contains(&dir) && dir.is_dir() {
            out.push(dir);
        }
    }
    out
}

/// The executable `name`: where the login shell finds it (made stable),
/// else the first of [`dirs`] that has it.
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
        .unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into())
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
    fn node_versions_sort_newest_first() {
        assert!(version_key("v22.23.1") > version_key("v22.9.0"));
        assert!(version_key("v24.0.0") > version_key("v22.23.1"));
        assert_eq!(version_key("system"), None);
    }
}
