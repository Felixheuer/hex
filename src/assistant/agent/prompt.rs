//! A task's prompts: the system prompt (who the assistant is, its lane's
//! and tools' rules), the rules only the request decides, and the message
//! a run sends.

use std::sync::OnceLock;

use super::*;

/// The system prompt of `job`'s run: who the assistant is, its lane's and
/// tools' rules, the routing or job rules, the autonomy mode, and the
/// memory note. Only the settings and the task decide it, never the words
/// of the request, so a backend started ahead of the question (`prewarm`)
/// or kept for the follow-up has exactly it, and the model's prompt cache
/// keeps it from one request to the next. What only the request decides
/// goes with its message ([`request_rules`]).
///
/// Every tool it names is one the run has, on every backend
/// (`toolset`): the guard's for the lane (`guard::proxy::tools`), Hex's for
/// the role (`tools::Role`), the browser's, and the backend's built-ins.
pub fn system_for(job: &Job, desktop: Option<&Desktop>, browser: bool) -> String {
    let settings = &job.settings;
    let origin = job.request.origin;
    let role = tool_role(job);
    let turn = role == crate::assistant::tools::Role::Turn;
    let mut system = system_prompt(origin, desktop, browser);
    match &job.workspace {
        Some(workspace) => {
            system.push_str(&job_rules(
                workspace,
                JobTools {
                    backend: settings.backend,
                    desktop: desktop.is_some(),
                    browser,
                    team: role == crate::assistant::tools::Role::Job,
                },
                settings.autonomy == crate::assistant::autonomy::Autonomy::Yolo,
            ));
            system.push_str(&project_rules(&settings.projects_folder));
        }
        // A chat runs in the Background lane: it reaches every app.
        None if origin != Origin::Background => system.push_str(&routing(
            desktop.is_some(),
            origin.lane() == Lane::Background || desktop.is_some_and(Desktop::reaches_apps),
            browser,
        )),
        None => {}
    }
    if let Some(lead) = &job.helps {
        system.push_str(&team::helper_rules(lead));
    }
    // The mode's baseline; what the person's words allowed on top of it is
    // the request's (`request_rules`).
    system.push_str(&autonomy_rules(
        settings.autonomy,
        &crate::assistant::autonomy::Authorized::default(),
    ));
    system.push_str(&speed_rules(
        settings.backend,
        desktop.is_some(),
        browser,
        !turn,
    ));
    // A job can't make schedules or remember (`tools::Role`); it still
    // reads the memory note.
    if turn {
        system.push_str(SCHEDULE_RULES);
    }
    system.push_str(crate::assistant::knowledge::RULES);
    if desktop.is_some() {
        system.push_str(super::rules::BACKGROUND_INPUT_RULES);
    }
    if settings.memory {
        system.push_str(&memory_rules(job.memory.as_deref(), turn));
    }
    system
}

/// What only this request decides, sent at the top of its message: the
/// person's language (pages, tools, and Hex's notes are mostly English, so
/// it is named), when their words or a yes allowed more than the mode's
/// baseline in the system prompt, what they allowed, and what the memory
/// graph knows about what their words name (`Job::recall`).
pub fn request_rules(job: &Job) -> String {
    let mode = job.settings.autonomy;
    let mut rules = language_rule(&job.request.text).trim().to_owned();
    let allowed = autonomy_rules(mode, &job.authorized);
    if allowed != autonomy_rules(mode, &crate::assistant::autonomy::Authorized::default()) {
        rules.push_str("\nFor this request: ");
        rules.push_str(allowed.trim());
    }
    if let Some(recall) = job
        .recall
        .as_deref()
        .map(str::trim)
        .filter(|recall| !recall.is_empty())
    {
        rules.push('\n');
        rules.push_str(recall);
    }
    rules
}

/// The message a run sends: [`request_rules`], then [`prompt`] with the
/// desktop as it is now.
pub(super) fn message(job: &Job, history: &[Message], with_history: bool) -> String {
    format!(
        "{}\n\n{}",
        request_rules(job),
        prompt_now(job, history, with_history)
    )
}

/// The message a run sends: the conversation so far when the backend has no
/// session for it (`with_history`), what else is going on for a new
/// conversation, the Here lane's window and what was on screen, then the
/// question, the new instruction (steering), or the note to go on.
pub fn prompt(job: &Job, history: &[Message], with_history: bool) -> String {
    prompt_with(job, history, with_history, None)
}

/// [`prompt`] with the desktop as it is now (`computer::now`), read fresh
/// for each run of a task that may use the desktop.
pub(super) fn prompt_now(job: &Job, history: &[Message], with_history: bool) -> String {
    let now = job
        .settings
        .control_desktop
        .then(|| crate::assistant::computer::now::for_prompt(&job.settings.blocked_apps))
        .flatten();
    prompt_with(job, history, with_history, now.as_deref())
}

/// [`prompt`], with `desktop_now` (the running apps and their windows)
/// before what was on screen and the question.
pub fn prompt_with(
    job: &Job,
    history: &[Message],
    with_history: bool,
    desktop_now: Option<&str>,
) -> String {
    let request = &job.request;
    let mut prompt = String::new();
    if with_history && !history.is_empty() {
        prompt.push_str("The conversation so far:\n");
        for message in history {
            let text = message.text();
            if text.trim().is_empty() {
                continue;
            }
            let who = if message.role == Role::User {
                "Person"
            } else {
                "You"
            };
            prompt.push_str(&format!("{who}: {}\n", text.trim()));
        }
        prompt.push('\n');
    }
    if history.is_empty()
        && let Some(digest) = job.digest.as_deref().filter(|digest| !digest.is_empty())
    {
        prompt.push_str(digest.trim_end());
        prompt.push_str("\n\n");
    }
    if let Some(now) = desktop_now.filter(|now| !now.trim().is_empty()) {
        prompt.push_str(now.trim_end());
        prompt.push_str("\n\n");
    }
    let context = &request.context;
    let mut seen = Vec::new();
    if let Some(app) = context
        .frontmost_app
        .as_deref()
        .filter(|app| !app.is_empty())
    {
        seen.push(format!("Frontmost app: {app}"));
    }
    if let Some(title) = context
        .window_title
        .as_deref()
        .filter(|title| !title.is_empty())
    {
        seen.push(format!("Its window: {title}"));
    }
    if request.origin.lane() == Lane::Here
        && let Some(window) = &context.window
    {
        let id = window
            .window_id
            .as_deref()
            .map(|id| format!(" (window id {id})"))
            .unwrap_or_default();
        // With full control every window is in reach; without it, only
        // the one in front.
        let full =
            crate::assistant::full_control(job.settings.background_front_window, &request.text);
        seen.push(if full {
            format!("The window in front: {}{id}", window.app)
        } else {
            format!("Your only window, the one in front: {}{id}", window.app)
        });
    }
    if let Some(display) = context
        .display
        .as_deref()
        .filter(|display| !display.is_empty())
    {
        seen.push(format!("Displays: {display}"));
    }
    if let Some(selected) = context
        .selected_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        seen.push(format!("Selected text:\n<<<\n{}\n>>>", selected.trim()));
    }
    if context.screenshot.is_some() {
        seen.push("A screenshot of the screen is attached.".into());
    }
    if !seen.is_empty() {
        prompt.push_str("What the person has on screen as they ask:\n");
        prompt.push_str(&seen.join("\n"));
        prompt.push_str("\n\n");
    }
    let text = request.text.trim();
    match &job.kind {
        JobKind::Ask => {
            if !prompt.is_empty() {
                prompt.push_str("The person asks: ");
            }
            prompt.push_str(text);
        }
        JobKind::Steer { doing, steps } => {
            let doing = doing.as_deref().unwrap_or("the task");
            prompt.push_str(&format!(
                "The person interrupted you while you were doing this: {doing}"
            ));
            if !steps.is_empty() {
                prompt.push_str(&format!(" (last steps: {})", steps.join("; ")));
            }
            prompt.push_str(&format!(
                ". Their new instruction: “{text}”. Adjust what you are doing; don't redo finished steps."
            ));
        }
        JobKind::AfterLimit => {
            prompt.push_str(&format!("{LIMIT_NOTE}\nThe task was: “{text}”"));
        }
        JobKind::Resume => {
            prompt.push_str(&format!("{RESUME_NOTE}\nThe task was: “{text}”"));
        }
        JobKind::Job { title } => {
            prompt.push_str(&format!(
                "Your job (“{title}”), as the person asked for it: {text}\nStart with the plan."
            ));
        }
        JobKind::Answer { question } => {
            prompt.push_str(&format!(
                "You asked: “{question}”\nThe person answered: “{text}”\nGo on with the job."
            ));
        }
        JobKind::Confirm { question, allowed } => {
            prompt.push_str(&format!(
                "You asked: “{question}”\nThe person answered: “{text}”\n{}",
                if *allowed {
                    "That's a yes: Hex now allows it for this task. Do it, then go on."
                } else {
                    "That's not a yes: skip that action, go on with the rest, and say at the end what you skipped."
                }
            ));
        }
    }
    prompt
}

/// What the person's computer is called in a prompt: "Mac", "Windows PC",
/// or "Linux computer".
pub fn computer_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mac"
    } else if cfg!(windows) {
        "Windows PC"
    } else {
        "Linux computer"
    }
}

/// The language every answer, question, and report is written in: the
/// request's. Words too short or mixed to tell ("open Safari") get "the
/// language of their words", never this Mac's: a person on a German Mac may
/// well speak English to it.
pub(super) fn language_rule(request: &str) -> String {
    let Some(code) = crate::assistant::speech::language_of(request) else {
        return LANGUAGE_OF_THEIR_WORDS.into();
    };
    let name = match code {
        "de" => "German",
        "en" => "English",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "nl" => "Dutch",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "uk" => "Ukrainian",
        "ru" => "Russian",
        "ja" => "Japanese",
        "zh" => "Chinese",
        _ => return LANGUAGE_OF_THEIR_WORDS.into(),
    };
    format!(
        "\nThe person speaks {name}: write every answer, question, and report to them in {name}, even when pages, tools, and Hex's notes are in English (messages to others in the language the request says).\n"
    )
}

/// The language rule when the request's language can't be named.
const LANGUAGE_OF_THEIR_WORDS: &str = "\nWrite every answer, question, and report to the person in the language of their words, even when pages, tools, and Hex's notes are in English (messages to others in the language the request says).\n";

/// The browser rules of a quick (Here) turn; when it is for (a quick look,
/// never research) is the routing's (`routing`).
pub const HERE_BROWSER: &str = "You have your own hidden browser (the browser tools); prefer it over the person's browser windows. Omit `session`; it starts with the person's accounts when there are any, and `signedIn: true` opens real Chrome for heavy web apps or logins the light one can't handle. Never solve or get around a captcha or a check that you are human: ask the person with a reply starting `HEX_QUESTION:` to solve it in the live picture of that browser on your card.\n";

/// Who the assistant is, how it answers, and its lane's rules for the
/// desktop and the browser.
pub fn system_prompt(origin: Origin, desktop: Option<&Desktop>, browser: bool) -> String {
    let mut prompt = format!(
        "You are Hex's assistant on the person's {}. Hex is a voice app: the person speaks a request and you answer or do it.\n",
        computer_name()
    );
    prompt.push_str(match origin {
        Origin::ActionShortcut => {
            "This is a quick spoken request, often about the window in front of the person: do it or answer it right away. Your answer is shown in a small bubble and read aloud, so keep it short (one to three sentences), plain, and speakable: no markdown, lists, tables, code blocks, links, or emoji. The question was transcribed from speech, so read past small transcription errors.\n"
        }
        Origin::Background => {
            "This is a background task, running beside the person and other tasks while they keep working. Do it, then end with one or two plain sentences saying what you did; no markdown.\n"
        }
        Origin::Chat => {
            "This is a typed chat; it may run beside other tasks. Markdown is fine; be as brief as the question allows.\n"
        }
    });
    // A quick turn's browser: the short form (what it is for: `routing`).
    if browser && origin.lane() == Lane::Here {
        prompt.push_str(HERE_BROWSER);
    } else if browser {
        prompt.push_str(&format!(
            "You have your own hidden browsers (the browser tools): prefer them for anything on the web (looking things up, web apps, forms), over the person's browser windows. Every browser tool takes `session` (omit it, or \"main\", for the main browser; any other short name opens another, at most {} on this computer). For research, or when the person asks for several browsers, open one per source or subtopic and work them in parallel (several tool calls in one turn), so the job is done fast; read pages in the light browsers, real Chrome only where a login or a heavy web app needs it. Your browsers start with the person's accounts (the sites they signed in to for Hex) when there are any, in a light browser; pass `signedIn: true` on the call that opens one for heavy web apps or logins that need real Chrome, or `signedIn: false` for a clean one with no accounts. Never ask the person to sign in unless a page still shows a login after that. If a page shows a captcha or a Cloudflare challenge, retry it in a `signedIn: true` browser. If it still asks you to prove you are human, never try to solve or get around it yourself: leave that browser open on the challenge, do whatever else needs no such page, then ask the person to solve it, starting your reply with the line `HEX_QUESTION:` (one or two speakable sentences naming the site, saying they can solve it in the live picture of that browser on your card: Take over, solve it, Hand back). When they answer, look at the page afresh and go on.\n",
            crate::assistant::browser::max_sessions()
        ));
    }
    let Some(desktop) = desktop else {
        prompt.push_str(
            "You have no access to the desktop or other apps in this conversation: answer from what you know and what the person gave you. Do not offer to click, type, or open apps.\n",
        );
        return prompt;
    };
    // What the desktop tools are, from the driver behind them.
    prompt.push('\n');
    prompt.push_str(
        &desktop
            .driver
            .backend()
            .intro()
            .replace("{computer}", computer_name()),
    );
    prompt.push_str("Use hex_installed_apps to see what's installed before choosing an app.\n");
    prompt.push_str(
        "Every desktop action passes Hex's guard, which lets you act only in windows that are yours. A refusal comes back as a tool error with a code: NOT_YOUR_WINDOW (another task's window, or one you haven't opened or claimed), PERSON_IN_WINDOW (the person is working there), TAKEN_OVER (the person took this task over; look again before acting), POLICY_DENIED (a blocked app, or an action tasks may not do). Don't try to get around a refusal: choose another way, or say what you couldn't do.\n",
    );
    if desktop.driver == crate::assistant::driver::Driver::Cua {
        prompt.push_str("There are no real-pointer tools: act through refs, or by a window's picture with hex_window_shot and hex_window_input.\n");
    } else if desktop.headed {
        prompt.push_str("Physical input (--headed) is allowed, but prefer headless actions; use physical input only when no semantic action exists.\n");
    } else {
        prompt.push_str("Use headless actions only: physical input is turned off, so actions that need --headed (hover, drag, mouse clicks at a point, double-click) are refused. Never try to switch to physical input.\n");
    }
    prompt.push_str(
        "Rules:\n\
         - Only do what the person asked, and do it: act instead of describing how or offering to. Never ask back what you can find out yourself (the message's desktop section, a snapshot, hex_installed_apps); if something only the person can tell is missing and there is no sensible default, ask for it (a job: in one message starting with `HEX_QUESTION:`) instead of guessing. Sending, deleting, paying, and posting follow the autonomy rules below.\n\
         - Which apps run and which windows are open: the message's \u{201c}desktop right now\u{201d} section is read fresh for this message; trust it over anything said earlier in the conversation or in other tasks' results (an app said to be open earlier may have been closed since). Use its window ids directly; list windows again only after something changed.\n\
         - Never quit or close an app you didn't open in this task.\n\
         - Read before acting: one snapshot of the window (interactive refs), act on a ref, then check only what matters.\n",
    );
    // Hex's app tools reach every app wherever the task may open apps.
    let reach = origin.lane() == Lane::Background || desktop.reaches_apps();
    if reach {
        prompt.push_str(APP_METHODS);
    }
    match origin.lane() {
        Lane::Here if desktop.front_window => prompt.push_str(
            "- The person gave you full control of this Mac: look at and act in any window no other task holds, the one in front included, by its window_id (it becomes yours as you act in it; no need to claim it first) and open or switch to any app with hex_open_app (it takes a running app's window into this task, reopens one that has no window, and never starts a second instance). Never say you can't open, reach, or use an app or window, and never offer a background job for something you can do now. Asked only to open an app, open it, say so in one short sentence, and ask what to do in it.\n",
        ),
        Lane::Here if desktop.open_apps => prompt.push_str(
            "- The person wants their own windows left alone: act in the window in front only if asked about it, and never in other windows they use. But open or switch to any app with hex_open_app: its window becomes yours, parked out of their way and live on your card, and you act there right away (a running app is never started twice). Never say you can't open or use an app, and never offer a background job for something you can do now. Asked only to open an app, open it, say so in one short sentence, and ask what to do in it.\n",
        ),
        Lane::Here => prompt.push_str(
            "- You may LOOK at any window to answer: list them with desktop_list_windows and read or screenshot the one the person means by its window_id (\"what am I doing in Citrix?\" means the Citrix window, even if it isn't in front; hex_read_window reads one that has no Accessibility tree). ACT only in the window in front, named in the message, and only if asked; don't bring another window forward.\n- If the request needs to act anywhere beyond that window (take over or operate another window such as Citrix, open an app or a browser, the web, files), never say you can't and never tell the person to do it themselves: hand it off at once with hex_start_jobs (see below), however small it is; a background job may open apps and browsers and take over the person's other windows.\n",
        ),
        Lane::Background => prompt.push_str(&format!(
            "- You run beside the person and other agents, each with its own cursor; yours is agent “{}”. Work only in your own windows: to use any app, even one that already runs, call hex_open_app first; it takes the app's window into this task (never a second instance), parks it out of the person's way, and shows it live on your card. Never only check that an app runs and report that. Or claim a free window that is already open with desktop_claim_window (its window_id from desktop_list_windows). {}Never another task's windows: don't move, close, or type into them. Once you know a window's id, always pass it (--window-id / window_id), and don't bring windows to the front.\n\
             - {}\n",
            desktop.agent_id,
            if desktop.front_window {
                "The person allowed you to use the window in front of them too: claim it like any other when the job needs it (for example their browser tab or document), act in it without bringing other windows forward, and leave it as you found it apart from what the job changes. "
            } else {
                "Never the person's window (the one in front). "
            },
            if browser {
                "Use the browser tools for anything on the web; use the desktop only for native apps."
            } else {
                "For web work, open your own browser window rather than using one the person has open."
            }
        )),
    }
    if origin.lane() == Lane::Here && reach {
        prompt.push_str(
            "- Done with an app you opened (\u{201c}close WhatsApp\u{201d}): let go of its window with hex_release_window; quit it (desktop_close_app) only when asked to.\n",
        );
    }
    if desktop.reaches_apps() {
        prompt.push_str(
            "- Background first: act in windows where they are, parked or behind others; never bring one to the front just to act in it. Only an app that doesn't react in the background after one retry gets its window forward for the shortest time (the person sees the frame); check the result, then give the front back to the window the person was in.\n",
        );
    }
    if desktop.driver == crate::assistant::driver::Driver::AgentDesktop
        && let Some(guide) = guide(desktop.executable())
    {
        prompt.push_str("\nagent-desktop's guide:\n\n");
        prompt.push_str(guide);
    }
    prompt
}

/// agent-desktop's own guide (`skills get desktop` and `platform`), read
/// once per run of Hex.
pub(super) fn guide(executable: &std::path::Path) -> Option<&'static str> {
    static GUIDE: OnceLock<Option<String>> = OnceLock::new();
    GUIDE
        .get_or_init(|| {
            let read = |args: &[&str]| -> Option<String> {
                let output = crate::assistant::backends::agent_command(executable)
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output()
                    .ok()?;
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
                value
                    .pointer("/data/content")
                    .and_then(serde_json::Value::as_str)
                    .map(strip_front_matter)
            };
            let desktop = read(&["skills", "get", "desktop"])?;
            let platform = read(&["skills", "get", "platform"]).unwrap_or_default();
            Some(trim_guide(&format!("{desktop}\n\n{platform}")))
        })
        .as_deref()
}

/// Sections of agent-desktop's guide written for a person or agent at a
/// shell, which a task never needs: it calls the tools (no shell, no
/// `--help`, no `skills get`), and Hex sets up the permissions.
pub(super) const GUIDE_SKIPPED: &[&str] = &[
    "Shell syntax",
    "First run",
    "References",
    "Requirements",
    "First contact",
];

/// The guide without [`GUIDE_SKIPPED`]'s `##` sections, every one up to
/// the next heading. A guide whose headings changed loses nothing.
pub(super) fn trim_guide(markdown: &str) -> String {
    let mut kept = String::with_capacity(markdown.len());
    let mut skipping = false;
    let mut fenced = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced && line.starts_with('#') {
            skipping = line
                .strip_prefix("## ")
                .is_some_and(|title| GUIDE_SKIPPED.contains(&title.trim()));
        }
        if !skipping {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept.trim().to_owned()
}

/// Markdown without its leading `---` front matter.
pub(super) fn strip_front_matter(markdown: &str) -> String {
    let trimmed = markdown.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        return rest[end + 4..].trim_start().to_owned();
    }
    trimmed.to_owned()
}
