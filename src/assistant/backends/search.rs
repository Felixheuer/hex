//! Where the agents' executables are, for a GUI app that never got the
//! person's shell PATH: the directories installers use, and the PATH a
//! session would have. On macOS and Linux that is the login shell's own
//! answer, asked once (bounded) and kept; Windows has no login shell, so
//! there it is the registry's `Path` (read afresh) and the process's, and a
//! name is found with each of `PATHEXT`'s endings.
//!
//! fnm puts a per-shell "multishell" directory first on PATH that is gone
//! once that shell closes; it is a link to an alias (`aliases/default`), and
//! that alias is what Hex keeps.

mod windows;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::os::Os;

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

/// What Windows runs a name with when `PATHEXT` isn't set.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

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
/// and kept until [`forget`]. Empty on Windows, which has none.
pub fn shell() -> Shell {
    if !Os::HERE.has_login_shell() {
        return Shell::default();
    }
    let mut cached = SHELL.lock().unwrap_or_else(|error| error.into_inner());
    cached.get_or_insert_with(ask_shell).clone()
}

/// Forgets the login shell's answer (after an install).
pub fn forget() {
    *SHELL.lock().unwrap_or_else(|error| error.into_inner()) = None;
}

/// The person's login shell: `$SHELL`, else their account's, else zsh on
/// macOS and sh elsewhere.
fn login_shell() -> PathBuf {
    let usable = |shell: &PathBuf| shell.is_absolute() && shell.exists();
    let fallback = if Os::HERE == Os::Mac {
        "/bin/zsh"
    } else {
        "/bin/sh"
    };
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(usable)
        .or_else(|| account_shell().filter(usable))
        .unwrap_or_else(|| PathBuf::from(fallback))
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

/// The folders installers use, as this session names them.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub home: PathBuf,
    /// `FNM_DIR`.
    pub fnm_dir: Option<PathBuf>,
    /// `XDG_DATA_HOME`.
    pub xdg_data: Option<PathBuf>,
    /// npm's global prefix (`NPM_CONFIG_PREFIX`, else `prefix=` in
    /// `~/.npmrc`), when set.
    pub npm_prefix: Option<PathBuf>,
    /// Windows: `%APPDATA%`, `%LOCALAPPDATA%`, `%ProgramFiles%`,
    /// `%ProgramData%`, `%SystemRoot%`, and nvm-windows' `%NVM_SYMLINK%`.
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub program_files: Option<PathBuf>,
    pub program_data: Option<PathBuf>,
    pub system_root: Option<PathBuf>,
    pub nvm_symlink: Option<PathBuf>,
}

impl Env {
    /// As this process sees it.
    fn here() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let home = home().unwrap_or_else(|| PathBuf::from("/"));
        let npm_prefix = var("NPM_CONFIG_PREFIX")
            .or_else(|| var("npm_config_prefix"))
            .or_else(|| {
                let text = std::fs::read_to_string(home.join(".npmrc")).ok()?;
                npmrc_prefix(&text, &home)
            });
        Self {
            fnm_dir: var("FNM_DIR"),
            xdg_data: var("XDG_DATA_HOME"),
            npm_prefix,
            app_data: var("APPDATA"),
            local_app_data: var("LOCALAPPDATA"),
            program_files: var("ProgramFiles"),
            program_data: var("ProgramData"),
            system_root: var("SystemRoot"),
            nvm_symlink: var("NVM_SYMLINK"),
            home,
        }
    }
}

/// The `prefix=` an `.npmrc` sets (`~` is `home`).
pub fn npmrc_prefix(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "prefix").then(|| value.trim().trim_matches('"').to_owned())
    })?;
    match value.strip_prefix("~/").or(value.strip_prefix("~\\")) {
        Some(rest) => Some(home.join(rest)),
        None if !value.is_empty() => Some(PathBuf::from(value)),
        None => None,
    }
}

/// `base` joined with each of `parts`, so a test of another OS's folders
/// reads the same on this one.
fn under(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

/// Where installers put these tools on `os`, most specific first;
/// directories that don't exist are left out by the caller.
pub fn known_dirs(os: Os, env: &Env) -> Vec<PathBuf> {
    match os {
        Os::Windows => windows_dirs(env),
        Os::Mac | Os::Linux => unix_dirs(os, env),
    }
}

fn unix_dirs(os: Os, env: &Env) -> Vec<PathBuf> {
    let home = &env.home;
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".opencode/bin"),
    ];
    if os == Os::Mac {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
    } else {
        // Homebrew on Linux, when someone uses it.
        dirs.extend([
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
            home.join(".linuxbrew/bin"),
        ]);
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.extend(env.npm_prefix.as_ref().map(|prefix| prefix.join("bin")));
    dirs.extend([
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
    ]);
    // fnm's default alias, wherever its data lives.
    let fnm_roots = [
        env.fnm_dir.clone(),
        env.xdg_data.as_ref().map(|data| data.join("fnm")),
        Some(home.join(".local/share/fnm")),
        (os == Os::Mac).then(|| home.join("Library/Application Support/fnm")),
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
    ]);
    dirs.push(PathBuf::from(if os == Os::Mac {
        "/opt/local/bin"
    } else {
        "/snap/bin"
    }));
    dirs
}

fn windows_dirs(env: &Env) -> Vec<PathBuf> {
    let home = &env.home;
    let in_dir =
        |base: &Option<PathBuf>, parts: &[&str]| base.as_ref().map(|base| under(base, parts));
    [
        // Claude Code's native installer.
        Some(under(home, &[".local", "bin"])),
        // npm's global folder, and a prefix of one's own.
        in_dir(&env.app_data, &["npm"]),
        env.npm_prefix.clone(),
        in_dir(&env.local_app_data, &["Microsoft", "WinGet", "Links"]),
        Some(under(home, &["scoop", "shims"])),
        in_dir(&env.program_data, &["chocolatey", "bin"]),
        in_dir(&env.program_files, &["nodejs"]),
        env.nvm_symlink.clone(),
        in_dir(&env.local_app_data, &["Volta", "bin"]),
        in_dir(&env.program_files, &["Volta"]),
        Some(under(home, &[".bun", "bin"])),
        // fnm's default alias holds node.exe itself.
        env.fnm_dir
            .as_ref()
            .map(|root| under(root, &["aliases", "default"])),
        in_dir(&env.app_data, &["fnm", "aliases", "default"]),
        in_dir(&env.local_app_data, &["fnm", "aliases", "default"]),
        Some(under(home, &[".opencode", "bin"])),
        in_dir(&env.local_app_data, &["Programs", "nodejs"]),
        Some(under(home, &[".cargo", "bin"])),
        Some(under(home, &[".deno", "bin"])),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The system's own directories, last.
pub fn system_dirs(os: Os, env: &Env) -> Vec<PathBuf> {
    match os {
        Os::Windows => {
            let root = env
                .system_root
                .clone()
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
            let mut dirs = vec![
                root.join("System32"),
                root.clone(),
                under(&root, &["System32", "WindowsPowerShell", "v1.0"]),
            ];
            dirs.extend(
                env.program_files
                    .as_ref()
                    .map(|files| under(files, &["PowerShell", "7"])),
            );
            dirs
        }
        Os::Mac | Os::Linux => ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
            .map(PathBuf::from)
            .into(),
    }
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

/// The PATH a fresh session has, before the known directories: the login
/// shell's (macOS; Linux adds the process's), or on Windows the registry's
/// and then the process's.
fn session_path() -> Vec<PathBuf> {
    let process = || {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    match Os::HERE {
        Os::Mac => shell().path,
        Os::Linux => shell().path.into_iter().chain(process()).collect(),
        Os::Windows => {
            #[cfg(windows)]
            let registry = windows::registry_path();
            #[cfg(not(windows))]
            let registry = Vec::new();
            registry.into_iter().chain(process()).collect()
        }
    }
}

/// Every directory to look in, in order: the session's PATH (made stable),
/// then the known install directories, then the system's. Only directories
/// that exist, each once.
pub fn dirs() -> Vec<PathBuf> {
    let env = Env::here();
    let mut out: Vec<PathBuf> = Vec::new();
    let all = session_path()
        .into_iter()
        .map(|dir| stable(&dir, read_link))
        .chain(known_dirs(Os::HERE, &env))
        .chain(nvm_dirs(&env.home))
        .chain(system_dirs(Os::HERE, &env));
    for dir in all {
        if !out.contains(&dir) && dir.is_dir() {
            out.push(dir);
        }
    }
    out
}

/// The files that could be `name` in `dir` on `os`: itself, or on Windows
/// each ending in `pathext` Hex can start (`.com`, `.exe`, `.bat`, `.cmd`,
/// in `PATHEXT`'s order) unless it already has one. An extensionless file
/// there is npm's shell script for Git Bash, which Windows can't run.
pub fn candidates(os: Os, dir: &Path, name: &str, pathext: &str) -> Vec<PathBuf> {
    if os != Os::Windows {
        return vec![dir.join(name)];
    }
    let runnable = [".com", ".exe", ".bat", ".cmd"];
    let endings: Vec<String> = pathext
        .split(';')
        .map(|ending| ending.trim().to_ascii_lowercase())
        .filter(|ending| runnable.contains(&ending.as_str()))
        .collect();
    let has_one = Path::new(name).extension().is_some_and(|ending| {
        let ending = format!(".{}", ending.to_string_lossy().to_ascii_lowercase());
        endings.contains(&ending)
    });
    if has_one {
        return vec![dir.join(name)];
    }
    endings
        .iter()
        .map(|ending| dir.join(format!("{name}{ending}")))
        .collect()
}

fn pathext() -> String {
    std::env::var("PATHEXT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PATHEXT.to_owned())
}

/// The executable `name`: where the login shell finds it (made stable),
/// else the first of [`dirs`] that has it.
pub fn find(name: &str) -> Option<PathBuf> {
    let shell = shell();
    let pathext = pathext();
    shell
        .found
        .iter()
        .filter(|(found, _)| found == name)
        .map(|(_, path)| stable(path, read_link))
        .find(|path| is_executable(path))
        .or_else(|| {
            dirs()
                .into_iter()
                .flat_map(|dir| candidates(Os::HERE, &dir, name, &pathext))
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
            let env = Env::here();
            let separator = Os::HERE.path_separator().to_string();
            system_dirs(Os::HERE, &env)
                .iter()
                .map(|dir| dir.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(&separator)
        })
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

    fn env(home: &str) -> Env {
        Env {
            home: PathBuf::from(home),
            ..Env::default()
        }
    }

    #[test]
    fn known_dirs_cover_the_installers_and_fnms_default_alias() {
        let mut mac = env("/Users/f");
        mac.fnm_dir = Some(PathBuf::from("/custom/fnm"));
        let dirs = known_dirs(Os::Mac, &mac);
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
    fn linux_looks_in_its_own_installers_folders_and_homebrews() {
        let mut linux = env("/home/f");
        linux.npm_prefix = Some(PathBuf::from("/home/f/.npm-packages"));
        linux.xdg_data = Some(PathBuf::from("/home/f/.data"));
        let dirs = known_dirs(Os::Linux, &linux);
        for dir in [
            "/home/f/.local/bin",
            "/home/f/.npm-packages/bin",
            "/home/linuxbrew/.linuxbrew/bin",
            "/home/f/.data/fnm/aliases/default/bin",
            "/snap/bin",
        ] {
            assert!(dirs.contains(&PathBuf::from(dir)), "{dir} is missing");
        }
        for mac_only in ["/opt/homebrew/bin", "/opt/local/bin"] {
            assert!(!dirs.contains(&PathBuf::from(mac_only)), "{mac_only}");
        }
        assert!(
            !dirs
                .iter()
                .any(|dir| dir.to_string_lossy().contains("Library"))
        );
    }

    #[test]
    fn windows_looks_where_its_installers_put_tools() {
        let home = PathBuf::from(r"C:\Users\f");
        let roaming = PathBuf::from(r"C:\Users\f\AppData\Roaming");
        let local = PathBuf::from(r"C:\Users\f\AppData\Local");
        let windows = Env {
            home: home.clone(),
            app_data: Some(roaming.clone()),
            local_app_data: Some(local.clone()),
            program_files: Some(PathBuf::from(r"C:\Program Files")),
            system_root: Some(PathBuf::from(r"C:\Windows")),
            ..Env::default()
        };
        let dirs = known_dirs(Os::Windows, &windows);
        for dir in [
            under(&home, &[".local", "bin"]),
            under(&roaming, &["npm"]),
            under(&local, &["Microsoft", "WinGet", "Links"]),
            under(&home, &["scoop", "shims"]),
            under(&PathBuf::from(r"C:\Program Files"), &["nodejs"]),
            under(&roaming, &["fnm", "aliases", "default"]),
        ] {
            assert!(dirs.contains(&dir), "{} is missing", dir.display());
        }
        assert!(
            !dirs
                .iter()
                .any(|dir| dir.to_string_lossy().starts_with('/'))
        );
        let system = system_dirs(Os::Windows, &windows);
        assert_eq!(
            system[0],
            under(&PathBuf::from(r"C:\Windows"), &["System32"])
        );
        assert!(system.contains(&under(
            &PathBuf::from(r"C:\Windows"),
            &["System32", "WindowsPowerShell", "v1.0"]
        )));
    }

    #[test]
    fn a_name_on_windows_is_found_with_pathexts_runnable_endings() {
        let dir = Path::new("npm");
        assert_eq!(
            candidates(
                Os::Windows,
                dir,
                "claude",
                ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.PS1"
            ),
            ["claude.com", "claude.exe", "claude.bat", "claude.cmd"].map(|name| dir.join(name))
        );
        // Already ending in one: only itself, never the extensionless shim.
        assert_eq!(
            candidates(Os::Windows, dir, "node.exe", DEFAULT_PATHEXT),
            [dir.join("node.exe")]
        );
        assert_eq!(
            candidates(Os::Mac, dir, "claude", DEFAULT_PATHEXT),
            [dir.join("claude")]
        );
        assert_eq!(
            candidates(Os::Linux, dir, "agent-desktop", ""),
            [dir.join("agent-desktop")]
        );
    }

    #[test]
    fn npms_own_prefix_is_read_from_npmrc() {
        let home = Path::new("/home/f");
        assert_eq!(
            npmrc_prefix("registry=https://r\nprefix = ~/.npm-global\n", home),
            Some(PathBuf::from("/home/f/.npm-global"))
        );
        assert_eq!(
            npmrc_prefix("prefix=\"/opt/npm\"", home),
            Some(PathBuf::from("/opt/npm"))
        );
        assert_eq!(npmrc_prefix("color=false", home), None);
    }

    #[test]
    fn node_versions_sort_newest_first() {
        assert!(version_key("v22.23.1") > version_key("v22.9.0"));
        assert!(version_key("v24.0.0") > version_key("v22.23.1"));
        assert_eq!(version_key("system"), None);
    }
}
