//! Starting a tool Hex found. On Windows npm installs each tool as a `.cmd`
//! script (a "cmd-shim") that runs `node <its script>`. A `.cmd` can't be
//! started like a program: Windows runs it through `cmd.exe`, whose command
//! line is cut at 8191 characters and can't carry a prompt's line breaks,
//! and an agent's MCP client won't start one at all. So a shim is read and
//! what it runs is started directly: `node.exe` with the script, or the
//! program it points at. Elsewhere a tool starts as it is.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::os::Os;

/// A program to start and the arguments that come before the caller's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// What a cmd-shim runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShimTarget {
    /// A script for `node`.
    Node(PathBuf),
    /// A program of its own.
    Program(PathBuf),
}

/// What the cmd-shim `text` in folder `dir` runs, as npm, pnpm, and Yarn
/// write them: `"%_prog%" "%dp0%\node_modules\…\cli.js" %*` for a script
/// (`_prog` is `node`), or `"%dp0%\…\tool.exe" %*` for a program. `None`
/// for anything else (npm's own `npm.cmd`, a script for another
/// interpreter).
pub fn shim_target(text: &str, dir: &Path) -> Option<ShimTarget> {
    let marker = ["\"%dp0%\\", "\"%~dp0\\", "\"%dp0%/"]
        .iter()
        .filter_map(|marker| text.rfind(marker).map(|at| (at, marker.len())))
        .max_by_key(|(at, _)| *at)?;
    let (at, length) = marker;
    let rest = &text[at + length..];
    let relative = &rest[..rest.find('"')?];
    let target = relative
        .split(['\\', '/'])
        .filter(|part| !part.is_empty() && *part != ".")
        .fold(dir.to_path_buf(), |path, part| {
            if part == ".." {
                path.parent().map(Path::to_path_buf).unwrap_or(path)
            } else {
                path.join(part)
            }
        });
    let line = text[..at].rsplit('\n').next().unwrap_or_default();
    if !line.contains("%_prog%") {
        return Some(ShimTarget::Program(target));
    }
    // The interpreter: the bare name `_prog` falls back to.
    let interpreter = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("SET \"_prog="))
        .map(|value| value.trim_end_matches('"'))
        .rfind(|value| !value.contains('%'))
        .unwrap_or("node");
    let interpreter = interpreter.trim_end_matches(".exe");
    interpreter
        .eq_ignore_ascii_case("node")
        .then_some(ShimTarget::Node(target))
}

/// How to start `path` on `os`: a Windows shim's target (with `node`, as
/// `node` finds it, for a script), else `path` itself.
pub fn launch(
    os: Os,
    path: &Path,
    read: impl Fn(&Path) -> Option<String>,
    node: impl Fn(&Path) -> Option<PathBuf>,
) -> Launch {
    let itself = || Launch {
        program: path.to_path_buf(),
        args: Vec::new(),
    };
    let is_script = path.extension().is_some_and(|ending| {
        ending.eq_ignore_ascii_case("cmd") || ending.eq_ignore_ascii_case("bat")
    });
    if os != Os::Windows || !is_script {
        return itself();
    }
    let Some(dir) = path.parent() else {
        return itself();
    };
    match read(path).and_then(|text| shim_target(&text, dir)) {
        Some(ShimTarget::Program(program)) => Launch {
            program,
            args: Vec::new(),
        },
        Some(ShimTarget::Node(script)) => match node(dir) {
            Some(node) => Launch {
                program: node,
                args: vec![script.into_os_string()],
            },
            None => itself(),
        },
        // Started as it is: the standard library runs a `.cmd` through
        // `cmd.exe`, quoting its arguments safely.
        None => itself(),
    }
}

/// [`launch`] for this OS: a shim's `node.exe` beside it (where npm puts
/// one), else the one on Hex's search path.
pub fn here(path: &Path) -> Launch {
    launch(
        Os::HERE,
        path,
        |path| std::fs::read_to_string(path).ok(),
        |dir| {
            let beside = dir.join("node.exe");
            if beside.is_file() {
                Some(beside)
            } else {
                super::search::find("node")
            }
        },
    )
}

/// A command for `program` that never shows a window, started the way
/// [`here`] says.
pub fn command(program: &Path) -> Command {
    let launch = here(program);
    let mut command = crate::desktop::background_process::command(&launch.program);
    command.args(&launch.args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPM_SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\cli.js\" %*\r\n";

    const PROGRAM_SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\agent-desktop\\bin\\agent-desktop.exe\"   %*\r\n";

    fn dir() -> PathBuf {
        PathBuf::from("C:").join("Users").join("f").join("npm")
    }

    #[test]
    fn an_npm_shim_runs_its_script_with_node() {
        assert_eq!(
            shim_target(NPM_SHIM, &dir()),
            Some(ShimTarget::Node(
                dir()
                    .join("node_modules")
                    .join("@anthropic-ai")
                    .join("claude-code")
                    .join("cli.js")
            ))
        );
        assert_eq!(
            shim_target(PROGRAM_SHIM, &dir()),
            Some(ShimTarget::Program(
                dir()
                    .join("node_modules")
                    .join("agent-desktop")
                    .join("bin")
                    .join("agent-desktop.exe")
            ))
        );
        // npm's own npm.cmd is no cmd-shim, nor is a script for sh.
        let npm = "@ECHO OFF\r\nSET \"NODE_EXE=%~dp0\\node.exe\"\r\n\"%NODE_EXE%\" \"%NPM_CLI_JS%\" %*\r\n";
        assert_eq!(shim_target(npm, &dir()), None);
        let sh = NPM_SHIM.replace("_prog=node", "_prog=sh");
        assert_eq!(shim_target(&sh, &dir()), None);
    }

    #[test]
    fn a_windows_shim_starts_node_and_everything_else_starts_as_it_is() {
        let shim = dir().join("claude.cmd");
        let node = dir().join("node.exe");
        let read = |_: &Path| Some(NPM_SHIM.to_owned());
        let launched = launch(Os::Windows, &shim, read, |_| Some(node.clone()));
        assert_eq!(launched.program, node);
        assert_eq!(launched.args.len(), 1);
        assert!(Path::new(&launched.args[0]).ends_with(Path::new("claude-code").join("cli.js")));
        // No node anywhere: the shim itself, through cmd.exe.
        let alone = launch(Os::Windows, &shim, read, |_| None);
        assert_eq!(alone.program, shim);
        // A program, and any other OS, start as they are.
        let exe = dir().join("claude.exe");
        assert_eq!(launch(Os::Windows, &exe, read, |_| None).program, exe);
        let unix = PathBuf::from("/usr/local/bin/claude.cmd");
        assert_eq!(launch(Os::Linux, &unix, read, |_| None).program, unix);
        assert!(launch(Os::Mac, &unix, read, |_| None).args.is_empty());
    }
}
