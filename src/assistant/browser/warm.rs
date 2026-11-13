//! Starting a task's browsers ahead: its main one as its first call would
//! start it, real Chrome with the person's accounts only for a request
//! that names a site they signed in to, and the marker a first call waits
//! on while one is still starting.

use super::*;

/// Starts `task`'s main browser now, exactly as its first browser call
/// would start it (`Sessions::default_launch`): a job's first page then
/// opens in a warm browser while its agent is still starting. Measured with
/// agent-browser 0.42 on this Mac: a cold first page took 0.31 s in
/// Lightpanda and 0.56–1.09 s in Chrome, a warm one 0.10 s and 0.12 s
/// (twenty helpers' at once 0.36 s, against about 4 s cold). Only a light
/// Lightpanda browser is started for every job, and only while memory is
/// fine; real Chrome with the person's accounts only when `request` names a
/// site they signed in to for Hex ([`names_signed_in_site`]): the task's
/// first `signedIn: true` browser is that one
/// (`Sessions::pin`). The task's end closes them like any other
/// ([`close`]), and they close themselves when unused for half an hour.
/// Runs on threads of its own; starting one that runs already changes
/// nothing (it reads the page's address and never navigates).
pub fn prewarm_for(task: &str, request: &str) {
    if cfg!(test) {
        return;
    }
    let enabled = crate::app_settings::assistant_runtime()
        .is_none_or(|(_, settings)| settings.background_browser);
    if !enabled
        || crate::assistant::capacity::pressure() != crate::assistant::capacity::Pressure::Normal
    {
        return;
    }
    let Some(path) = executable() else {
        return;
    };
    let sessions =
        sessions::Sessions::new(task, clean_engine()).with_accounts(accounts_by_default());
    let mut starts = Vec::new();
    if clean_engine() == Engine::Lightpanda {
        starts.push((session(task), sessions.default_launch()));
    }
    if signed_in() && names_signed_in_site(request) {
        let (name, launch) = sessions.warm_chrome();
        starts.push((name, launch));
    }
    for (session, launch) in starts {
        let path = path.clone();
        // The task's first browser call waits for this start instead of
        // racing it (two starts of one session: the second fails).
        let warming = Warming::mark(&session);
        let spawned = std::thread::Builder::new()
            .name("hex-browser-prewarm".into())
            .spawn(move || {
                let _warming = warming;
                let started = std::time::Instant::now();
                let (args, env) = prewarm_command(&launch);
                let mut command = command(&path, &session);
                command.envs(env).arg("--json").args(&args);
                match run::bounded(command, COMMAND_LIMIT) {
                    Ok(ran) if !ran.timed_out() => tracing::debug!(
                        session,
                        ms = started.elapsed().as_millis() as u64,
                        "started a job's browser ahead"
                    ),
                    Ok(_) | Err(_) => {
                        tracing::debug!(session, "couldn't start a job's browser ahead")
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::debug!(%error, "couldn't start a browser ahead");
        }
    }
}

/// Whether `request` names a site the person signed in to for Hex (its
/// address, or its name: "github" for github.com), so a job is worth a
/// warm real Chrome with their accounts. Read from the sign-in store, never
/// a list of sites; nothing in tests, which never read the person's.
pub fn names_signed_in_site(request: &str) -> bool {
    if cfg!(test) {
        return false;
    }
    names_site(request, &signed_in_sites())
}

/// Whether `request` names one of `sites` (hosts such as
/// `mail.example.com`): the host itself, or the site's own name, the label
/// before its suffix (`example`), as a whole word of at least four letters.
pub fn names_site(request: &str, sites: &[String]) -> bool {
    let text = request.to_lowercase();
    let words: std::collections::HashSet<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '.' && c != '-')
        .map(|word| word.trim_matches(|c: char| c == '.' || c == '-'))
        .filter(|word| !word.is_empty())
        .collect();
    sites.iter().any(|site| {
        let host = site.trim_start_matches('.').to_lowercase();
        if host.is_empty() {
            return false;
        }
        if words.contains(host.as_str()) {
            return true;
        }
        site_name(&host).is_some_and(|name| name.chars().count() >= 4 && words.contains(name))
    })
}

/// A host's own name: the label before its suffix (`example` in
/// `mail.example.com` and in `shop.example.co.uk`).
pub(super) fn site_name(host: &str) -> Option<&str> {
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    let n = labels.len();
    if n < 2 {
        return None;
    }
    // A two-letter country suffix under a short second level (co.uk, com.au).
    let suffix = if n >= 3 && labels[n - 1].len() == 2 && labels[n - 2].len() <= 3 {
        2
    } else {
        1
    };
    labels.get(n.checked_sub(suffix + 1)?).copied()
}

/// The sites in the sign-in store, read again only when a state file
/// changed.
fn signed_in_sites() -> Vec<String> {
    type Stamp = Vec<(std::ffi::OsString, Option<std::time::SystemTime>, u64)>;
    static CACHE: std::sync::Mutex<Option<(Stamp, Vec<String>)>> = std::sync::Mutex::new(None);
    let mut stamp: Stamp = state_dir()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_sign_in_state(&entry.file_name().to_string_lossy()))
        .map(|entry| {
            let meta = entry.metadata().ok();
            (
                entry.file_name(),
                meta.as_ref().and_then(|meta| meta.modified().ok()),
                meta.map_or(0, |meta| meta.len()),
            )
        })
        .collect();
    stamp.sort();
    if let Ok(cache) = CACHE.lock()
        && let Some((seen, sites)) = cache.as_ref()
        && *seen == stamp
    {
        return sites.clone();
    }
    let sites = saved_sites();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((stamp, sites.clone()));
    }
    sites
}

/// The agent-browser arguments and environment that start a browser with
/// `launch` (`Sessions::default_launch`'s `restore`, `restoreSave`, and
/// `extraArgs`) and only read its page's address: on a browser that runs
/// already they change nothing, so a prewarm that comes late never takes a
/// task's page away.
pub(super) fn prewarm_command(
    launch: &serde_json::Map<String, Value>,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut args: Vec<String> = launch
        .get("extraArgs")
        .and_then(Value::as_array)
        .map(|extra| {
            extra
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    args.extend(["get".to_owned(), "url".to_owned()]);
    let mut env = Vec::new();
    for (key, name) in [
        ("restore", "AGENT_BROWSER_RESTORE"),
        ("restoreSave", "AGENT_BROWSER_RESTORE_SAVE"),
    ] {
        if let Some(value) = launch.get(key).and_then(Value::as_str) {
            env.push((name.to_owned(), value.to_owned()));
        }
    }
    (args, env)
}

/// A browser being started ahead (`prewarm_for`): a marker file under
/// `<support>/assistant/browser-warming/<session>` while it starts, which
/// the task's browser proxy (another process) waits on before its first
/// call to that session. Removed when dropped.
pub(crate) struct Warming(Option<PathBuf>);

/// How long a first call waits for a start ahead, at most.
const WARM_WAIT: Duration = Duration::from_secs(20);

impl Warming {
    fn dir() -> Option<PathBuf> {
        crate::app_paths::support_dir()
            .ok()
            .map(|support| support.join("assistant").join("browser-warming"))
    }

    fn mark(session: &str) -> Self {
        let path = Self::dir().map(|dir| dir.join(session));
        if let Some(path) = &path {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new("/")));
            let _ = std::fs::write(path, std::process::id().to_string());
        }
        Self(path)
    }
}

impl Drop for Warming {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Waits while `session` is being started ahead (at most [`WARM_WAIT`],
/// and never on a marker older than a start can take).
pub(crate) fn wait_for_warmup(session: &str) {
    let Some(path) = Warming::dir().map(|dir| dir.join(session)) else {
        return;
    };
    let started = std::time::Instant::now();
    loop {
        let fresh = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < COMMAND_LIMIT);
        if !fresh || started.elapsed() >= WARM_WAIT {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
