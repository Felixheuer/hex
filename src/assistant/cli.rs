//! `agent-desk assistant` (hidden): runs one assistant task here, without the app,
//! and prints its events as they come (with how long the first token and
//! the first spoken sentence took); lists and shows saved conversations;
//! resumes an interrupted one; and asks a running Agent-Desk to stop all tasks or
//! to let the person take one over. For trying backends end to end. It runs
//! even while the assistant is off, because a person runs it on purpose.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Subcommand, ValueEnum};
use color_eyre::{Result, eyre::eyre};

use super::service::{self, remote};
use super::types::{Context, Event, Lane, Origin, Part, Request, Role, TaskWindow};
use crate::app_settings::AssistantBackend;

#[derive(Subcommand, Debug)]
pub enum AssistantCommand {
    /// Show how much voice a barge-in needs on this Mac, as learnt from the
    /// last barge-ins, and how they turned out.
    BargeIn {
        /// Forget what was learnt: back to the defaults.
        #[arg(long)]
        reset: bool,
    },
    /// Every tool call a conversation's agent made (shell commands, files,
    /// clicks, pages, messages), as a timeline.
    Audit {
        /// The conversation's id (`agent-desk assistant conversations`).
        id: String,
    },
    /// Ask one question and print what happens until it's answered.
    Ask {
        text: String,
        /// The agent to use; the saved choice by default.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
        /// The backend's model; the saved one by default.
        #[arg(long)]
        model: Option<String>,
        /// OpenCode with this provider's saved API key (`deepseek`,
        /// `openrouter`, …; `agent-desk assistant provider-key`); the saved
        /// choice by default.
        #[arg(long)]
        provider: Option<String>,
        /// Its lane: `here` (the window in front; the default) or
        /// `background` (its own windows, beside other tasks).
        #[arg(long, value_enum, conflicts_with = "chat")]
        lane: Option<LaneArg>,
        /// Same as `--lane background`.
        #[arg(long, conflicts_with_all = ["chat", "lane"])]
        agent: bool,
        /// Run it as a typed Chats message, not a quick question.
        #[arg(long)]
        chat: bool,
        /// The Here lane's window: the app in front (its running name).
        #[arg(long)]
        window_app: Option<String>,
        /// The Here lane's window id (`w-<CoreGraphics window number>`).
        #[arg(long, requires = "window_app")]
        window_id: Option<String>,
        /// Start the backend this many seconds before asking, as the
        /// Action shortcut does while the person speaks.
        #[arg(long, value_name = "SECONDS")]
        prewarm: Option<f64>,
        /// Then ask this follow-up in the same conversation.
        #[arg(long, value_name = "TEXT")]
        then: Option<String>,
        /// Don't let it use apps (no desktop tools).
        #[arg(long)]
        no_desktop: bool,
        /// Read a quick answer aloud.
        #[arg(long)]
        speak: bool,
        /// Continue this conversation.
        #[arg(long = "continue", value_name = "ID")]
        conversation: Option<String>,
    },
    /// List saved conversations, newest first.
    Conversations,
    /// Print one saved conversation.
    Show { id: String },
    /// Save the Anthropic API key read from standard input in the Keychain
    /// (an empty input deletes it).
    Key,
    /// Save an API key for OpenCode (bring your own key) read from
    /// standard input, then check it with one tiny request: `deepseek`,
    /// `openrouter`, `openai`, … (`--list` names them all). An empty input
    /// deletes it.
    ProviderKey {
        /// The provider's id.
        #[arg(required_unless_present = "list")]
        provider: Option<String>,
        /// List the providers and whether a key is saved for each.
        #[arg(long)]
        list: bool,
        /// Only check the key already saved.
        #[arg(long, conflicts_with = "list")]
        check: bool,
    },
    /// Continue a conversation Agent-Desk quit in the middle of, and print what
    /// happens.
    Resume { id: String },
    /// Ask the running Agent-Desk to stop every assistant task.
    StopAll,
    /// Ask the running Agent-Desk to pause a task and bring its window forward.
    TakeOver { id: String },
    /// Ask the running Agent-Desk to give a task back after `take-over`.
    HandBack { id: String },
    /// Cua Driver, the desktop driver: its version and update status
    /// (`status`, the default), a check for a newer release now (`update`),
    /// or back to the previous version (`rollback`).
    Driver {
        #[command(subcommand)]
        command: Option<DriverCommand>,
    },
    /// List, add, remove, or watch scheduled jobs.
    Schedules {
        #[command(subcommand)]
        command: Option<SchedulesCommand>,
    },
    /// Agent-Desk's own tools for a task's agent (memory, schedules) as an MCP
    /// server on stdio. Started by the task's agent.
    #[command(hide = true)]
    ToolsMcp {
        #[arg(long)]
        task: String,
        /// `job` for a background job (team tools; no memory, schedules,
        /// or hand-off), `helper` for a job's helper (the same, without a
        /// team), `turn` (the default) for a question or a chat.
        #[arg(long, default_value = "turn")]
        role: String,
        /// The team's board (`team.md` in the lead's folder), for a job
        /// and its helpers.
        #[arg(long)]
        board: Option<std::path::PathBuf>,
    },
    /// The guard's MCP server for one task (Agent-Desk gives it to each task's
    /// agent in place of the desktop driver): `--spec` is the task's JSON spec or
    /// a file holding it.
    #[command(hide = true)]
    Mcp {
        #[arg(long)]
        spec: String,
    },
    /// A task's hidden browser as an MCP server on stdio (agent-browser's
    /// own, pinned to the task's session). Started by the task's agent.
    #[command(hide = true)]
    BrowserMcp {
        #[arg(long)]
        task: String,
        /// A job's check: no consequential action, whatever the mode.
        #[arg(long)]
        observe: bool,
    },
    /// Closes every hidden browser a task opened.
    #[command(hide = true)]
    BrowserClose {
        #[arg(long)]
        task: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum DriverCommand {
    /// The installed version and how the last update check went.
    Status,
    /// Looks for a newer Cua Driver and installs it now: checked, tested,
    /// switched when no task acts (it waits up to `--wait` seconds).
    Update {
        #[arg(long, default_value_t = 120)]
        wait: u64,
    },
    /// Goes back to the version before the current one.
    Rollback,
    /// The person's own browser (Chrome, Edge, Brave) through Cua Driver's
    /// browser tools: whether tasks may attach it (`--attach` allows it and
    /// attaches a running one now, Chrome asks once; `--off` stops it).
    /// Without it tasks use the person's tabs through hex_browser_tab.
    Browser {
        #[arg(long)]
        attach: bool,
        #[arg(long, conflicts_with = "attach")]
        off: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SchedulesCommand {
    /// Every schedule with its next run (the default).
    List,
    /// Schedule a job: `--when "0 8 * * 1-5"` or an RRULE.
    Add {
        #[arg(long)]
        when: String,
        /// What to do each time.
        task: String,
    },
    /// Remove a schedule.
    Remove {
        id: String,
    },
    /// Turn a schedule on or off.
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Run due schedules here for a while, printing what happens (for
    /// trying them without the app).
    Watch {
        #[arg(long, default_value_t = 2.0)]
        minutes: f64,
        /// Don't let the runs use apps (no desktop tools).
        #[arg(long)]
        no_desktop: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LaneArg {
    Here,
    Background,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Backend {
    ClaudeCode,
    Codex,
    #[value(name = "opencode")]
    OpenCode,
    ClaudeApi,
}

impl From<Backend> for AssistantBackend {
    fn from(backend: Backend) -> Self {
        match backend {
            Backend::ClaudeCode => Self::ClaudeCode,
            Backend::Codex => Self::Codex,
            Backend::OpenCode => Self::OpenCode,
            Backend::ClaudeApi => Self::ClaudeApi,
        }
    }
}

/// Runs `command`; the process's exit code.
pub fn run(command: AssistantCommand) -> u8 {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENT_DESK_ASSISTANT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();
    let result = match command {
        AssistantCommand::Ask {
            text,
            backend,
            model,
            provider,
            lane,
            agent,
            chat,
            window_app,
            window_id,
            prewarm,
            then,
            no_desktop,
            speak,
            conversation,
        } => ask(Ask {
            text,
            backend: backend.map(Into::into),
            model,
            provider,
            origin: if agent || matches!(lane, Some(LaneArg::Background)) {
                Origin::Background
            } else if chat {
                Origin::Chat
            } else {
                Origin::ActionShortcut
            },
            window: window_app.map(|app| TaskWindow { app, window_id }),
            prewarm: prewarm.map(Duration::from_secs_f64),
            then,
            no_desktop,
            speak,
            conversation,
        }),
        AssistantCommand::Resume { id } => resume(&id),
        AssistantCommand::StopAll => send(remote::ALL, remote::Verb::Stop),
        AssistantCommand::Audit { id } => {
            print!("{}", super::audit::report(&super::audit::read(&id)));
            Ok(true)
        }
        AssistantCommand::BargeIn { reset } => {
            if reset {
                super::interaction::tuning::reset();
            }
            print!("{}", super::interaction::tuning::report());
            if reset {
                println!("Agent-Desk picks the defaults up the next time it starts.");
            }
            Ok(true)
        }
        AssistantCommand::TakeOver { id } => send(&id, remote::Verb::TakeOver),
        AssistantCommand::HandBack { id } => send(&id, remote::Verb::HandBack),
        AssistantCommand::Schedules { command } => schedules(command),
        AssistantCommand::Driver { command } => driver(command),
        AssistantCommand::ToolsMcp { task, role, board } => {
            super::tools::serve(&task, super::tools::Role::parse(&role), board).map(|()| true)
        }
        AssistantCommand::Conversations => {
            list();
            Ok(true)
        }
        AssistantCommand::Show { id } => show(&id).map(|()| true),
        AssistantCommand::Key => key().map(|()| true),
        AssistantCommand::ProviderKey {
            provider,
            list,
            check,
        } => provider_key(provider.as_deref(), list, check),
        AssistantCommand::Mcp { spec } => super::guard::serve(&spec).map(|()| true),
        AssistantCommand::BrowserMcp { task, observe } => {
            super::browser::serve(&task, observe).map(|()| true)
        }
        AssistantCommand::BrowserClose { task } => {
            super::browser::close_now(&task);
            Ok(true)
        }
    };
    match result {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(error) => {
            eprintln!("agent-desk: {error:#}");
            1
        }
    }
}

/// `agent-desk assistant driver …`.
fn driver(command: Option<DriverCommand>) -> Result<bool> {
    use super::driver::update;
    match command.unwrap_or(DriverCommand::Status) {
        DriverCommand::Status => {
            let status = update::status();
            println!("{}", status.line());
            if let Some(path) = super::driver::find() {
                println!("{}", path.display());
            }
            if let Some(latest) = &status.latest {
                println!("latest release: {latest}");
            }
            if let Some(error) = &status.error {
                println!("last error: {error}");
            }
            // The tools it offers, and what is new since the version before
            // (checked once per version).
            if let Some(home) = super::driver::cua::standard()
                && home.installed().is_some()
            {
                let used = super::guard::driver_tools_used();
                match super::driver::capabilities::refresh(
                    &home,
                    &super::guard::driver_probe,
                    crate::desktop::events::now_ms(),
                ) {
                    Ok(capabilities) => {
                        for line in capabilities.lines(&used) {
                            println!("{line}");
                        }
                    }
                    Err(why) => println!("tools: not checked ({why})"),
                }
                println!("{}", super::driver::browser::line(&home));
            }
            Ok(super::driver::find().is_some())
        }
        DriverCommand::Update { wait } => {
            // Agent-Desk's own tasks run in its process: only what every
            // process sees (leases, real input) says they act.
            let status = update::check_now(
                &update::idle_across,
                Duration::from_secs(wait),
                &mut |step| eprintln!("{step}"),
            )?;
            println!("{}", status.line());
            if let Some(error) = &status.error {
                println!("{error}");
            }
            Ok(status.outcome != update::Outcome::Failed)
        }
        DriverCommand::Rollback => {
            let back = update::roll_back()?;
            println!("Cua Driver {back} runs again.");
            Ok(true)
        }
        DriverCommand::Browser { attach, off } => {
            use super::driver::browser;
            let home = super::driver::cua::standard()
                .ok_or_else(|| color_eyre::eyre::eyre!("no application support folder"))?;
            if off {
                browser::allow(&home, false)?;
            }
            if attach {
                match browser::attach(&home) {
                    Ok(lines) => lines.iter().for_each(|line| println!("{line}")),
                    Err(why) => {
                        println!("{why}");
                        return Ok(false);
                    }
                }
            }
            println!("{}", browser::line(&home));
            Ok(true)
        }
    }
}

struct Ask {
    text: String,
    backend: Option<AssistantBackend>,
    model: Option<String>,
    provider: Option<String>,
    origin: Origin,
    window: Option<TaskWindow>,
    prewarm: Option<Duration>,
    then: Option<String>,
    no_desktop: bool,
    speak: bool,
    conversation: Option<String>,
}

/// Runs one task (and its follow-up); whether it was answered.
fn ask(ask: Ask) -> Result<bool> {
    service::run_from_cli();
    let Ask {
        backend,
        model,
        provider,
        no_desktop,
        speak,
        ..
    } = ask;
    if let Some(provider) = &provider
        && super::backends::provider(provider).is_none()
    {
        return Err(eyre!(
            "Agent-Desk doesn't know the provider “{provider}” (`agent-desk assistant provider-key --list`)"
        ));
    }
    service::adjust_settings(move |settings| {
        if let Some(backend) = backend {
            settings.backend = backend;
        }
        if let Some(provider) = &provider {
            settings.backend = AssistantBackend::OpenCode;
            settings.opencode_provider = Some(provider.clone());
        }
        if let Some(model) = &model {
            match settings.backend {
                AssistantBackend::ClaudeCode => settings.claude_code_model = Some(model.clone()),
                AssistantBackend::Codex => settings.codex_model = Some(model.clone()),
                AssistantBackend::OpenCode => settings.opencode_model = Some(model.clone()),
                AssistantBackend::ClaudeApi => settings.claude_api_model = model.clone(),
            }
            settings.here_model = Some(model.clone());
        }
        if no_desktop {
            settings.control_desktop = false;
        }
        settings.speak_answers = speak;
    });
    let (sender, events) = mpsc::channel();
    service::subscribe(Box::new(move |event| {
        let _ = sender.send(event.clone());
    }));
    let _ = ctrlc::set_handler(service::stop_all);
    let lane = ask.origin.lane();
    // A Here question is about the window in front, read as the Action
    // shortcut reads it (Accessibility only, no screenshot).
    let context = match &ask.window {
        Some(window) => Context {
            frontmost_app: Some(window.app.clone()),
            window: Some(window.clone()),
            ..Context::default()
        },
        None if lane == Lane::Here => super::computer::quick_context(),
        None => Context::default(),
    };
    let window = context.window.clone();
    if let Some(window) = &window {
        println!("[window in front] {} {:?}", window.app, window.window_id);
    }
    if let Some(ahead) = ask.prewarm {
        service::prewarm_for(lane, window.clone());
        std::thread::sleep(ahead);
    }
    let id = service::submit(Request {
        conversation: ask.conversation,
        text: ask.text,
        origin: ask.origin,
        context: context.clone(),
    });
    println!("conversation {id}");
    let mut answered = follow(&id, &events, speak);
    if let Some(then) = ask.then.filter(|_| answered) {
        if let Some(ahead) = ask.prewarm {
            service::prewarm_for(lane, window.clone());
            std::thread::sleep(ahead);
        }
        let again = service::submit(Request {
            conversation: Some(id.clone()),
            text: then,
            origin: ask.origin,
            context,
        });
        println!("follow-up {again}");
        answered = follow(&again, &events, speak);
    }
    service::stop_all();
    Ok(answered)
}

/// Prints `id`'s events until it ends, with timings; whether it was
/// answered. When the answer is read aloud, waits for it to be read.
fn follow(id: &str, events: &mpsc::Receiver<Event>, speak: bool) -> bool {
    let started = Instant::now();
    let mut first_token = None;
    let mut first_speech = None;
    let mut answered = false;
    let mut mid_line = false;
    for event in events {
        if event
            .conversation()
            .is_some_and(|conversation| conversation != id)
        {
            continue;
        }
        if mid_line && !matches!(event, Event::AnswerDelta { .. }) {
            println!();
            mid_line = false;
        }
        #[allow(clippy::wildcard_enum_match_arm)]
        match &event {
            Event::Paused { .. } => println!("[paused]"),
            Event::Resumed { .. } => println!("[resumed]"),
            Event::Waiting {
                reason, until_ms, ..
            } => {
                println!("[waiting until {until_ms}] {reason}")
            }
            Event::HandsOff { on, .. } => println!("[hands off] {on}"),
            Event::Borrowed { window, why, .. } => {
                println!("[in front] {} · {why}", window.app)
            }
            Event::Returned { to, .. } => {
                println!("[back to you] {}", to.as_deref().unwrap_or_default())
            }
            Event::Acting { .. } => {}
            Event::Board { claimed, done, .. } => {
                println!("[team board] {claimed} claimed, {done} done")
            }
            Event::InProject { name, .. } => println!("[project] {name}"),
            Event::ProjectSuggested { name, .. } => println!("[project?] {name}"),
            Event::Plan { steps, .. } => {
                for step in steps {
                    println!(
                        "[plan] {} {}",
                        if step.done { "✓" } else { "·" },
                        step.title
                    );
                }
            }
            Event::Question { text, .. } => {
                println!("[question] {text}");
                // The answer: a line from standard input, as a hold would
                // give it.
                let mut answer = String::new();
                let _ = std::io::stdin().read_line(&mut answer);
                if answer.trim().is_empty() {
                    service::cancel(id);
                } else {
                    println!("[answering] {}", answer.trim());
                    service::submit(Request {
                        conversation: Some(id.to_owned()),
                        text: answer.trim().to_owned(),
                        origin: Origin::ActionShortcut,
                        context: Context::default(),
                    });
                }
            }
            Event::Escalated { .. } => println!("[escalated] continues in the background"),
            Event::Queued { .. } => println!("[queued]"),
            Event::Started { origin, .. } => println!("[started] {origin:?}"),
            Event::Thinking { .. } => println!("[thinking]"),
            Event::Step { description, .. } => println!("[step] {description}"),
            Event::Window { window, .. } => match &window.window_id {
                Some(id) => println!("[window] {} ({id})", window.app),
                None => println!("[window] {}", window.app),
            },
            Event::AnswerDelta { text, .. } => {
                first_token.get_or_insert_with(|| started.elapsed());
                if speak && first_speech.is_none() && super::speech::speaking() {
                    first_speech = Some(started.elapsed());
                }
                if !mid_line {
                    print!("[delta] ");
                }
                print!("{text}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                mid_line = true;
            }
            Event::Answer { text, .. } => {
                first_token.get_or_insert_with(|| started.elapsed());
                println!("[answer] {text}");
                answered = true;
            }
            Event::Failed { message, .. } => println!("[failed] {message}"),
            Event::Cancelled { .. } => println!("[cancelled]"),
            Event::ConversationsChanged => println!("[saved]"),
            Event::Understood { text, .. } => println!("[understood] {text}"),
            Event::QuestionsChanged { active, waiting } => match active {
                Some(asked) => println!(
                    "[questions] {}: {} (+{waiting})",
                    asked.title, asked.question
                ),
                None => println!("[questions] none"),
            },
        }
        if event.is_final() {
            break;
        }
    }
    if speak && answered {
        // Wait for the reading to start and end.
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if super::speech::speaking() {
                first_speech.get_or_insert_with(|| started.elapsed());
            } else if first_speech.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let ms = |time: Option<Duration>| {
        time.map_or_else(|| "-".to_owned(), |time| format!("{} ms", time.as_millis()))
    };
    println!(
        "[timing] first token {}, first spoken word {}, total {} ms",
        ms(first_token),
        ms(first_speech),
        started.elapsed().as_millis()
    );
    answered
}

/// Resumes an interrupted conversation and prints what happens.
fn resume(id: &str) -> Result<bool> {
    service::run_from_cli();
    let (sender, events) = mpsc::channel();
    service::subscribe(Box::new(move |event| {
        let _ = sender.send(event.clone());
    }));
    let _ = ctrlc::set_handler(service::stop_all);
    let id = service::resume_interrupted(id)
        .ok_or_else(|| eyre!("“{id}” wasn't interrupted, or there is no such conversation"))?;
    println!("conversation {id}");
    Ok(follow(&id, &events, false))
}

fn schedules(command: Option<SchedulesCommand>) -> Result<bool> {
    use super::schedules;
    match command.unwrap_or(SchedulesCommand::List) {
        SchedulesCommand::List => {
            let list = schedules::list();
            if list.is_empty() {
                println!("No scheduled jobs.");
            }
            for schedule in list {
                let next = schedule
                    .next_due()
                    .map_or_else(|| "never".into(), schedules::describe);
                println!(
                    "{}  {:<3}  {:<24}  next {next}  {}",
                    schedule.id,
                    if schedule.enabled { "on" } else { "off" },
                    schedule.when,
                    schedule.task
                );
            }
            Ok(true)
        }
        SchedulesCommand::Add { when, task } => {
            let schedule = schedules::add(&when, &task, Lane::Background)?;
            let next: Vec<String> = schedules::next_runs(&schedule.when, 3)?
                .into_iter()
                .map(schedules::describe)
                .collect();
            println!("{}  next: {}", schedule.id, next.join(", "));
            Ok(true)
        }
        SchedulesCommand::Remove { id } => Ok(schedules::remove(&id)),
        SchedulesCommand::Enable { id } => Ok(schedules::set_enabled(&id, true)),
        SchedulesCommand::Disable { id } => Ok(schedules::set_enabled(&id, false)),
        SchedulesCommand::Watch {
            minutes,
            no_desktop,
        } => {
            service::run_from_cli();
            if no_desktop {
                service::adjust_settings(|settings| settings.control_desktop = false);
            }
            let (sender, events) = mpsc::channel();
            service::subscribe(Box::new(move |event| {
                let _ = sender.send(event.clone());
            }));
            let _ = ctrlc::set_handler(service::stop_all);
            let until = Instant::now() + Duration::from_secs_f64(minutes * 60.0);
            let mut ran = false;
            while Instant::now() < until {
                for id in service::run_due_schedules() {
                    println!("[scheduled run] conversation {id}");
                    ran |= follow(&id, &events, false);
                }
                std::thread::sleep(Duration::from_secs(5));
            }
            service::stop_all();
            Ok(ran)
        }
    }
}

/// Asks every running Agent-Desk to do `verb` to `target`.
fn send(target: &str, verb: remote::Verb) -> Result<bool> {
    remote::send(target, verb, Duration::from_secs(2))?;
    println!("Asked the running Agent-Desk; an Agent-Desk that runs no such task does nothing.");
    Ok(true)
}

fn list() {
    for summary in service::conversations() {
        println!(
            "{}  {:<26} {}  {}",
            summary.id,
            format!("{:?}/{:?}", summary.origin, summary.outcome),
            summary.title,
            summary.preview
        );
    }
}

fn show(id: &str) -> Result<()> {
    let conversation = service::conversation(id).ok_or_else(|| eyre!("no conversation “{id}”"))?;
    println!("{} ({:?})", conversation.title, conversation.origin);
    for message in &conversation.messages {
        let who = if message.role == Role::User {
            "You"
        } else {
            "Assistant"
        };
        for part in &message.parts {
            match part {
                Part::Text { text } => println!("{who}: {text}"),
                Part::Screenshot { file } => println!("{who}: [screenshot {file}]"),
                Part::Action { description } => println!("{who}: [{description}]"),
                Part::Error { message } => println!("{who}: [error] {message}"),
            }
        }
    }
    Ok(())
}

/// `provider-key`: saves (or with `check` only checks) a provider's key,
/// or lists the providers; whether all went well.
fn provider_key(provider: Option<&str>, list: bool, check: bool) -> Result<bool> {
    use super::backends;
    if list {
        for provider in backends::providers() {
            let saved = if backends::has_provider_key(provider.id) {
                "key saved"
            } else {
                "no key"
            };
            println!(
                "{:<12} {:<18} {saved:<10} {}/{}",
                provider.id, provider.name, provider.id, provider.default_model
            );
        }
        return Ok(true);
    }
    let id = provider.unwrap_or_default();
    let Some(provider) = backends::provider(id) else {
        return Err(eyre!(
            "Agent-Desk doesn't know the provider “{id}” (`agent-desk assistant provider-key --list`)"
        ));
    };
    if !check {
        let mut key = String::new();
        std::io::stdin().read_line(&mut key)?;
        backends::set_provider_key(provider.id, &key)?;
        if key.trim().is_empty() {
            println!("The saved {} key was deleted.", provider.name);
            return Ok(true);
        }
        println!("The {} key is saved.", provider.name);
    }
    match backends::verify_provider_key(provider.id) {
        Ok(found) => {
            println!("{found}.");
            Ok(true)
        }
        Err(why) => {
            println!("{why}.");
            Ok(false)
        }
    }
}

fn key() -> Result<()> {
    let mut key = String::new();
    std::io::stdin().read_line(&mut key)?;
    super::credentials::set_api_key(&key)?;
    println!(
        "{}",
        if key.trim().is_empty() {
            "The saved API key was deleted."
        } else {
            "The API key is saved in the Keychain."
        }
    );
    Ok(())
}
