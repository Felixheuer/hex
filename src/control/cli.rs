//! The `agent-desk` control client: parses a command, sends it to the running Agent-Desk,
//! and prints the answer for a person or, with `--json`, the versioned reply.
//!
//! Exit codes: 0 when the command ran (or there was nothing to change), 1
//! when Agent-Desk refused or failed it, 2 for usage errors (clap), 3 when Agent-Desk
//! isn't running, and 4 when `dictation stop --print` or `agent-desk transcribe`
//! heard no speech. Nothing here launches Agent-Desk except `start` and `restart`.

use std::io::Write;
mod human;
mod transcribe;

use clap::{Args, Subcommand};

use super::{
    CONTROL_VERSION, CaptureAction, ControlCommand, ControlReply, ControlRequest, ControlResponse,
    Pane,
};

pub const EXIT_REFUSED: u8 = 1;
pub const EXIT_NOT_RUNNING: u8 = 3;
pub const EXIT_NO_SPEECH: u8 = 4;

#[derive(Args, Clone, Copy, Debug, Default)]
pub struct Output {
    /// Print the versioned JSON reply (see docs/cli.md).
    #[arg(long)]
    pub json: bool,
}

/// The control commands at the top of `agent-desk`.
#[derive(Subcommand, Debug)]
pub enum ControlCli {
    /// Show what the running Agent-Desk is doing.
    ///
    /// Recording, meeting, model, features, permissions, window, version,
    /// pid, and memory footprint.
    Status {
        #[command(flatten)]
        output: Output,
    },
    /// Open Agent-Desk if it isn't running, and wait until it answers.
    Start {
        #[command(flatten)]
        output: Output,
    },
    /// Quit Agent-Desk.
    ///
    /// Refused while a meeting or dictation is in progress unless --force.
    Quit {
        /// Quit even while recording; the recording is lost.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Quit and reopen Agent-Desk (or open it if it isn't running).
    Restart {
        /// Restart even while recording; the recording is lost.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Start, stop, toggle, or cancel a dictation, as the shortcut would.
    ///
    /// `start` locks the recording: `stop` or the shortcut finishes it,
    /// `cancel` discards it. Repeating a command changes nothing.
    Dictation {
        #[arg(value_enum)]
        action: CaptureAction,
        /// With `start` (macOS) or `stop`:
        /// answer with the transcript instead of pasting it. A printing start
        /// waits for its recording to end. Exits 4, printing nothing, when no
        /// speech was heard.
        #[arg(long)]
        print: bool,
        /// With `start` or `toggle`: paste the transcript as heard,
        /// skipping the Output pane's steps, like Dictate without
        /// processing.
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Paste the last dictation at the current focus (Option-Shift-V).
    PasteLast {
        #[command(flatten)]
        output: Output,
    },
    /// Run the focused field's text through its Output steps and replace it
    /// (Linux).
    ///
    /// Sends Ctrl+A and Ctrl+C, so it is not meant for terminals.
    TypedText {
        #[command(flatten)]
        output: Output,
    },
    /// Transcribe a WAV file with the selected model and print it.
    ///
    /// Nothing is pasted, saved to History, or processed.
    Transcribe {
        /// The WAV file to transcribe.
        file: std::path::PathBuf,
        /// A model id from `agent-desk model list` (default: the selected model).
        #[arg(long)]
        model: Option<String>,
        /// A language the model supports (default: the saved language).
        #[arg(long)]
        language: Option<String>,
        #[command(flatten)]
        output: Output,
    },
    /// Retired with the Screenshots feature; answers that it is gone.
    #[command(hide = true)]
    Screenshot {
        /// Open the area and window picker instead, as its shortcut does.
        #[arg(long)]
        pick: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Read and change any persisted setting, shortcuts included.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// List, switch, install, and import transcription models.
    #[command(visible_alias = "models")]
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Open the Agent-Desk window at a pane, or close it.
    Window {
        #[command(subcommand)]
        command: WindowCommand,
    },
    /// Replay the first-run onboarding.
    Onboarding {
        #[command(subcommand)]
        command: OnboardingCommand,
    },
    /// Check for an Agent-Desk update now; Agent-Desk shows what it finds.
    CheckForUpdates {
        #[command(flatten)]
        output: Output,
    },
    /// Show Launch at Login, or turn it on or off.
    LaunchAtLogin {
        #[arg(value_enum)]
        state: Option<Toggle>,
        #[command(flatten)]
        output: Output,
    },
    /// List, add, and remove vocabulary terms.
    Vocabulary {
        #[command(subcommand)]
        command: VocabularyCommand,
    },
    /// Read retained dictation history without starting the app, or clear
    /// it in the running Agent-Desk (`agent-desk history clear --yes`).
    #[command(args_conflicts_with_subcommands = true)]
    History(crate::control::transcript_cli::HistoryArgs),
    /// Read and search recorded meetings without starting the app; record
    /// and stop them in the running Agent-Desk.
    Meetings {
        #[command(subcommand)]
        command: crate::control::transcript_cli::MeetingsCommand,
    },
    /// Serve recorded meetings to agents over MCP (stdio). Read-only.
    Mcp {
        /// Read meetings from this folder instead of Application Support, such
        /// as a mirrored copy on another computer.
        #[arg(long)]
        meetings_dir: Option<std::path::PathBuf>,
    },
    /// Manage the ~/.config/agent-desk personal transformations workspace.
    Commands {
        #[command(subcommand)]
        command: CommandsCommand,
    },
    /// The assistant: ask one question here and print what happens, or
    /// list saved conversations.
    #[command(hide = true)]
    Assistant {
        #[command(subcommand)]
        command: crate::assistant::cli::AssistantCommand,
    },
    /// Agent-Desk's benchmark: ask ~25 typical tasks of the running Agent-Desk Dev,
    /// check them by machine, and compare with the run before
    /// (docs/bench.md).
    Bench {
        #[command(subcommand)]
        command: crate::bench::BenchCommand,
    },
    /// Annotate: record your screen with drawing and narration in the
    /// running Agent-Desk, then list and send what you recorded (`agent-desk capture`
    /// still works).
    #[command(name = "annotate", alias = "capture")]
    Annotate {
        #[command(subcommand)]
        command: CaptureCommand,
    },
    /// Print Agent-Desk's recent log lines; works without Agent-Desk running.
    Logs {
        /// Read process.log (Rust and native diagnostics) instead of the
        /// live.ndjson event log.
        #[arg(long)]
        process: bool,
        /// How many lines from the end.
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
        /// Keep printing new lines until interrupted.
        #[arg(short, long)]
        follow: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CommandsCommand {
    /// Create ~/.config/agent-desk with a starter config, or refresh its typings.
    Init,
    /// Load ~/.config/agent-desk's config as Agent-Desk would and report what it registers
    /// or why it fails.
    Check,
}

#[derive(Subcommand, Debug)]
pub enum SettingsCommand {
    /// Every setting with its value, kind, and whether it can be set.
    List {
        #[command(flatten)]
        output: Output,
    },
    /// One setting's value, or every setting under a prefix (`meetings`).
    Get {
        key: String,
        #[command(flatten)]
        output: Output,
    },
    /// Change a setting as the window would.
    ///
    /// Booleans take on/off, shortcuts take text such as `option+shift+v`,
    /// `right-command`, `fn`, or `f5`, and optional settings take `none`.
    Set {
        key: String,
        value: String,
        #[command(flatten)]
        output: Output,
    },
    /// Set several settings at once from a JSON object of `key: value`,
    /// all or none; other settings keep their values.
    ///
    /// Values are JSON (`true`, `0.4`, `"option+space"`, `null` for none);
    /// `transcription.model` and `transcription.language` select a model
    /// without downloading it. With Agent-Desk stopped, writes settings.json
    /// directly.
    Apply {
        /// A JSON file, or `-` for standard input.
        file: std::path::PathBuf,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Subcommand, Debug)]
pub enum ModelCommand {
    /// Every model with its languages, size, and whether it's installed.
    List {
        #[command(flatten)]
        output: Output,
    },
    /// Switch to an installed model. Never downloads.
    Use {
        /// A model id from `agent-desk model list`.
        model: String,
        /// A language the model supports (default: its best match for the
        /// current language).
        #[arg(long)]
        language: Option<String>,
        #[command(flatten)]
        output: Output,
    },
    /// Download a model, then switch to it. Without a model, the selected
    /// one. On Linux and Windows it downloads here when Agent-Desk isn't running.
    Install {
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[command(flatten)]
        output: Output,
    },
    /// Stop a download or load in progress.
    Cancel {
        #[command(flatten)]
        output: Output,
    },
    /// Whether a model (the selected one unless given) is installed and
    /// checksum-verified. Runs here; Linux and Windows.
    Status {
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
    },
    /// The transcribe.cpp compute devices Agent-Desk can use. Linux and Windows.
    Devices,
    /// Load and prewarm an installed model here. Linux and Windows.
    Check {
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
    },
    /// Transcribe a WAV file with an installed model here. Linux and Windows.
    Transcribe {
        wav: std::path::PathBuf,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
    },
    /// Copy a GGUF speech model into Agent-Desk after checking its header,
    /// and print its id. The original is left as it is.
    Import {
        path: std::path::PathBuf,
        /// The name the picker shows (default: the file's general.name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Rename an imported or found model.
    Rename { model: String, name: String },
    /// Check Agent-Desk's copy of an import (or a found file) again; a copy whose contents
    /// still match is ready again.
    Repair { model: String },
    /// Remove Agent-Desk's copy of an imported model, or forget a found one (its file stays).
    Remove { model: String },
}

#[derive(Subcommand, Debug)]
pub enum OnboardingCommand {
    /// Open the first-run onboarding in the window now, as on a new install.
    ///
    /// Forgets that onboarding finished. Settings, history, models, and permissions stay,
    /// so their steps pass at once.
    #[command(visible_alias = "restart")]
    Reset {
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Subcommand, Debug)]
pub enum WindowCommand {
    /// Open the window, at a pane if given.
    Show {
        #[arg(value_enum)]
        pane: Option<Pane>,
        #[command(flatten)]
        output: Output,
    },
    /// Close the window. Agent-Desk keeps running in the menu bar.
    Hide {
        #[command(flatten)]
        output: Output,
    },
    /// Dev builds: open the window on a playground story by its path, name, or
    /// slug (`Prototypes/HUD/Processing`, `prototypes-hud-processing`).
    Story {
        story: String,
        /// The variant, by number from 1 or by name.
        #[arg(long)]
        variant: Option<String>,
        /// Run the story's `r` trigger once it's open, as pressing R does.
        #[arg(long)]
        play: bool,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Subcommand, Debug)]
pub enum VocabularyCommand {
    /// Every term with its "sounds like" spellings.
    List {
        #[command(flatten)]
        output: Output,
    },
    /// Add a term, or more spellings to a term already in the list.
    Add {
        term: String,
        /// Spellings a transcript may show instead, comma-separated
        /// (`--sounds-like "capuchin,cap a chino"`).
        #[arg(long, value_name = "SPELLINGS")]
        sounds_like: Option<String>,
        #[command(flatten)]
        output: Output,
    },
    /// Remove a term and its spellings.
    Remove {
        term: String,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Toggle {
    On,
    Off,
}

/// Exits with `code` once stdout is flushed.
pub fn finish(code: u8) -> ! {
    let _ = std::io::stdout().flush();
    std::process::exit(code.into())
}

pub fn run(command: ControlCli) -> u8 {
    match command {
        ControlCli::Status { output } => send(ControlCommand::Status, output),
        ControlCli::Start { output } => start(output),
        ControlCli::Quit { force, output } => send(ControlCommand::Quit { force }, output),
        ControlCli::Restart { force, output } => {
            if !running() {
                #[cfg(target_os = "linux")]
                if crate::linux::service::instance_answers() {
                    return print(restart_older_hex(), output);
                }
                return start(output);
            }
            #[cfg(target_os = "linux")]
            if let Err(error) = crate::linux::service::prepare_restart() {
                return print(ControlReply::error("failed", format!("{error:#}")), output);
            }
            #[cfg(target_os = "macos")]
            return send(ControlCommand::Restart { force }, output);
            #[cfg(not(target_os = "macos"))]
            match call(ControlRequest::new(ControlCommand::Restart { force })) {
                Ok(ControlResponse {
                    reply: ControlReply::Done { .. },
                    ..
                }) => print(reopen_after_restart(), output),
                Ok(response) => print(response.reply, output),
                Err(reply) => print(reply, output),
            }
        }
        ControlCli::Dictation {
            action,
            print,
            raw,
            output,
        } => {
            if let Err(error) = super::check_flags(action, print, raw) {
                eprintln!("agent-desk: {}", error.message);
                return 2;
            }
            send(ControlCommand::Dictation { action, print, raw }, output)
        }
        ControlCli::PasteLast { output } => send(ControlCommand::PasteLast, output),
        ControlCli::TypedText { output } => send(ControlCommand::TypedText, output),
        ControlCli::Transcribe {
            file,
            model,
            language,
            output,
        } => transcribe::run(file, model, language, output),
        ControlCli::Screenshot { pick, output } => {
            send(ControlCommand::Screenshot { pick }, output)
        }
        ControlCli::Settings { command } => settings(command),
        ControlCli::Model { command } => match command {
            ModelCommand::List { output } => send(ControlCommand::ModelList, output),
            ModelCommand::Use {
                model,
                language,
                output,
            } => send(ControlCommand::ModelUse { model, language }, output),
            ModelCommand::Install {
                model,
                language,
                output,
            } => install_model(model, language, output),
            ModelCommand::Cancel { output } => send(ControlCommand::ModelCancel, output),
            ModelCommand::Import { path, name } => report(import_model(&path, name.as_deref())),
            ModelCommand::Rename { model, name } => report(imported(&model).and_then(|id| {
                crate::transcription::custom_models::rename(id, &name)?;
                println!("renamed\t{}", id.as_str());
                Ok(())
            })),
            ModelCommand::Repair { model } => report(imported(&model).and_then(|id| {
                crate::transcription::custom_models::repair(
                    id,
                    &std::sync::atomic::AtomicBool::new(false),
                )?;
                println!("ready\t{}", id.as_str());
                Ok(())
            })),
            ModelCommand::Remove { model } => report(imported(&model).and_then(|id| {
                let settings = crate::app_settings::AppSettings::load().unwrap_or_default();
                if settings.transcription.model
                    == crate::transcription::models::TranscriptionModelId::Custom(id)
                {
                    color_eyre::eyre::bail!(
                        "{} is the selected model; choose another with `agent-desk model use` first",
                        id.as_str()
                    );
                }
                crate::transcription::custom_models::remove(id)?;
                println!("removed\t{}", id.as_str());
                Ok(())
            })),
            command => offline_model(command),
        },
        ControlCli::Window { command } => match command {
            WindowCommand::Show { pane, output } => {
                send(ControlCommand::WindowShow { pane }, output)
            }
            WindowCommand::Hide { output } => send(ControlCommand::WindowHide, output),
            WindowCommand::Story {
                story,
                variant,
                play,
                output,
            } => send(
                ControlCommand::WindowStory {
                    story,
                    variant,
                    play,
                },
                output,
            ),
        },
        ControlCli::Onboarding {
            command: OnboardingCommand::Reset { output },
        } => send(ControlCommand::OnboardingReset, output),
        ControlCli::CheckForUpdates { output } => send(ControlCommand::CheckForUpdates, output),
        ControlCli::LaunchAtLogin { state, output } => send(
            ControlCommand::LaunchAtLogin {
                enabled: state.map(|state| matches!(state, Toggle::On)),
            },
            output,
        ),
        ControlCli::Vocabulary { command } => match command {
            VocabularyCommand::List { output } => send(ControlCommand::VocabularyList, output),
            VocabularyCommand::Add {
                term,
                sounds_like,
                output,
            } => send(
                ControlCommand::VocabularyAdd {
                    term,
                    sounds_like: sounds_like.into_iter().collect(),
                },
                output,
            ),
            VocabularyCommand::Remove { term, output } => {
                send(ControlCommand::VocabularyRemove { term }, output)
            }
        },
        ControlCli::Logs {
            process,
            lines,
            follow,
        } => logs(process, lines, follow),
        ControlCli::History(crate::control::transcript_cli::HistoryArgs {
            action: Some(crate::control::transcript_cli::HistoryAction::Clear { yes, output }),
            ..
        }) => clear_history(yes, output),
        ControlCli::History(args) => report(crate::control::transcript_cli::history(&args)),
        ControlCli::Meetings { command } => meetings(command),
        ControlCli::Mcp { meetings_dir } => report(mcp(meetings_dir)),
        ControlCli::Commands { command } => report(match command {
            CommandsCommand::Init => crate::personal_commands::initialize_workspace()
                .map(|workspace| println!("{}", workspace.display())),
            CommandsCommand::Check => {
                crate::personal_commands::check_workspace().map(|report| println!("{report}"))
            }
        }),
        ControlCli::Assistant { command } => crate::assistant::cli::run(command),
        ControlCli::Bench { command } => crate::bench::run(command),
        ControlCli::Annotate { command } => capture(command),
    }
}

/// `agent-desk model install`: the running Agent-Desk downloads in the background;
/// without one, Linux and Windows download and select here.
fn install_model(model: Option<String>, language: Option<String>, output: Output) -> u8 {
    let model = match model {
        Some(model) => model,
        None => match crate::app_settings::AppSettings::read() {
            Ok(settings) => settings.transcription.model.as_str().to_owned(),
            Err(error) => return fail(&format!("couldn't read settings: {error:#}")),
        },
    };
    #[cfg(not(target_os = "macos"))]
    if !running() {
        let selection = match selection(Some(model), language) {
            Ok(selection) => selection,
            Err(error) => return fail(&format!("{error:#}")),
        };
        if let Err(error) = crate::transcription::local_transcriber::install(
            &selection,
            &std::sync::atomic::AtomicBool::new(false),
        ) {
            return fail(&format!("{error:#}"));
        }
        return select_installed(&selection, output);
    }
    send(ControlCommand::ModelInstall { model, language }, output)
}

/// `model install` with no Agent-Desk running: make the switch the app would have
/// made. An Agent-Desk that started while the model downloaded takes the choice;
/// otherwise it is saved under the service lock `settings apply` uses, so a
/// starting Agent-Desk is never written behind. An unchanged selection needs neither.
#[cfg(not(target_os = "macos"))]
fn select_installed(
    selection: &crate::transcription::models::TranscriptionSelection,
    output: Output,
) -> u8 {
    if crate::app_settings::AppSettings::read()
        .is_ok_and(|settings| settings.transcription == *selection)
    {
        let name = crate::transcription::models::definition(selection.model).name;
        println!("{name} ({}) is already selected", selection.language);
        return 0;
    }
    let handoff = hand_off(
        || select_in_running_hex(selection, output),
        || match save_selection(selection) {
            Ok(message) => {
                println!("{message}");
                0
            }
            Err(error) => fail(&format!(
                "the model is installed, but selecting it failed: {error:#}"
            )),
        },
    );
    match handoff {
        Handoff::Done(code) => code,
        #[cfg(target_os = "linux")]
        Handoff::OlderHex => fail(&format!(
            "the model is installed, but an older Agent-Desk is still running; \
             run `agent-desk restart`, then `agent-desk model use {}`",
            selection.model.as_str()
        )),
        Handoff::Deadline => fail(&format!(
            "the model is installed, but Agent-Desk is starting and holds its settings; \
             run `agent-desk model use {}` once it answers",
            selection.model.as_str()
        )),
    }
}

/// Hands a downloaded model to an Agent-Desk that started while it downloaded.
#[cfg(not(target_os = "macos"))]
fn select_in_running_hex(
    selection: &crate::transcription::models::TranscriptionSelection,
    output: Output,
) -> u8 {
    send(
        ControlCommand::ModelUse {
            model: selection.model.as_str().to_owned(),
            language: Some(selection.language.clone()),
        },
        output,
    )
}

/// Records a downloaded model as the selected one, as the app's own switch
/// does. An unchanged selection is left alone.
#[cfg(not(target_os = "macos"))]
fn save_selection(
    selection: &crate::transcription::models::TranscriptionSelection,
) -> color_eyre::Result<String> {
    let name = crate::transcription::models::definition(selection.model).name;
    let mut settings = crate::app_settings::AppSettings::load()?;
    if settings.transcription == *selection {
        return Ok(format!(
            "{name} ({}) is already selected",
            selection.language
        ));
    }
    settings.save_transcription(selection.clone())?;
    Ok(format!("Switched to {name} ({})", selection.language))
}

/// `agent-desk model import`: checks, copies, and records the file here, in this
/// process; a running Agent-Desk lists it at once.
fn import_model(path: &std::path::Path, name: Option<&str>) -> color_eyre::Result<()> {
    use crate::transcription::custom_models;
    let id = custom_models::import(path, name, &std::sync::atomic::AtomicBool::new(false))?;
    let definition = custom_models::definition(id);
    println!("imported\t{}\t{}", id.as_str(), definition.name);
    Ok(())
}

/// The import or found model in use that `model` names.
fn imported(model: &str) -> color_eyre::Result<crate::transcription::models::CustomModelId> {
    use crate::transcription::custom_models;
    crate::transcription::models::CustomModelId::parse(model)
        .filter(|id| custom_models::is_registered(*id))
        .ok_or_else(|| {
            color_eyre::eyre::eyre!("no imported model is {model}; run `agent-desk model list`")
        })
}

/// The selection a local model command names: `model` with its remembered
/// language unless `language` is given, or the selected model.
#[cfg(not(target_os = "macos"))]
fn selection(
    model: Option<String>,
    language: Option<String>,
) -> color_eyre::Result<crate::transcription::models::TranscriptionSelection> {
    let settings = crate::app_settings::AppSettings::read().unwrap_or_default();
    let model = model.unwrap_or_else(|| settings.transcription.model.as_str().to_owned());
    super::model_selection(&settings, &model, language.as_deref())
        .map_err(|error| color_eyre::eyre::eyre!(error.message))
}

/// `agent-desk model status|devices|check|transcribe`, run in this process.
#[cfg(not(target_os = "macos"))]
fn offline_model(command: ModelCommand) -> u8 {
    use crate::transcription::models::{definition, is_installed, is_verified, model_path};
    report((|| match command {
        ModelCommand::Status { model, language } => {
            let selection = selection(model, language)?;
            let model = definition(selection.model);
            let installed = is_installed(model, &selection.language);
            println!(
                "{}\t{}\t{}\t{}",
                if installed { "installed" } else { "missing" },
                if installed && is_verified(model) {
                    "verified"
                } else {
                    "unverified"
                },
                model.name,
                model_path(model)?.display()
            );
            Ok(())
        }
        ModelCommand::Devices => {
            for device in crate::transcription::local_transcriber::devices()? {
                println!("{device}");
            }
            Ok(())
        }
        ModelCommand::Check { model, language } => {
            let transcriber = crate::transcription::local_transcriber::LocalTranscriber::load(
                &selection(model, language)?,
            )?;
            println!("ready\t{}", transcriber.device_label());
            Ok(())
        }
        ModelCommand::Transcribe {
            wav,
            model,
            language,
        } => {
            println!(
                "{}",
                crate::transcription::local_transcriber::transcribe_wav_with_selection(
                    &wav,
                    &selection(model, language)?
                )?
            );
            Ok(())
        }
        _ => unreachable!("Agent-Desk carries out the other model commands"),
    })())
}

/// macOS runs models only inside the app.
#[cfg(target_os = "macos")]
fn offline_model(_: ModelCommand) -> u8 {
    print(
        ControlReply::error(
            "unsupported",
            "this `agent-desk model` command is for Linux and Windows; use `agent-desk model list`",
        ),
        Output::default(),
    )
}

/// The exit code of a command that runs here rather than in Agent-Desk.
fn report(result: color_eyre::Result<()>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(error) => fail(&format!("{error:#}")),
    }
}

/// `agent-desk meetings`: record, stop, and copy in the running Agent-Desk; the rest
/// read the meetings folder here.
fn meetings(command: crate::control::transcript_cli::MeetingsCommand) -> u8 {
    use crate::control::transcript_cli::MeetingsCommand;
    match command {
        MeetingsCommand::Record { title, output } => {
            send(ControlCommand::MeetingRecord { title }, output)
        }
        MeetingsCommand::Stop { output } => send(ControlCommand::MeetingStop, output),
        MeetingsCommand::CopyLast { output } => send(ControlCommand::MeetingCopyLast, output),
        command => report(crate::control::transcript_cli::meetings(&command)),
    }
}

/// `agent-desk mcp`: logs to stderr only (stdout carries MCP messages) and touches
/// no app state.
fn mcp(meetings_dir: Option<std::path::PathBuf>) -> color_eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AGENT_DESK_MCP_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let root = match meetings_dir {
        Some(directory) => directory,
        None => crate::meeting::root()?,
    };
    crate::meeting::mcp::run(root)
}

/// `agent-desk annotate` (alias `agent-desk capture`).
#[derive(Subcommand, Debug)]
pub enum CaptureCommand {
    /// Start recording a capture.
    Start {
        /// Linux: record this area instead of the screen, as X,Y,WIDTH,HEIGHT in pixels (on
        /// X11; Wayland's portal records the screen).
        #[cfg(target_os = "linux")]
        #[arg(long, value_delimiter = ',', value_name = "X,Y,W,H")]
        area: Option<Vec<f64>>,
        #[command(flatten)]
        output: Output,
    },
    /// End the capture and write it in the background.
    Stop {
        #[command(flatten)]
        output: Output,
    },
    /// End the capture and discard it.
    Cancel {
        #[command(flatten)]
        output: Output,
    },
    /// Stop the capture that records, else start one (for a key that fires
    /// on the press only).
    Toggle {
        #[command(flatten)]
        output: Output,
    },
    /// What records, what is being written, and the last capture.
    Status {
        #[command(flatten)]
        output: Output,
    },
    /// Export the last 15 s from the rolling screen buffer into a capture
    /// bundle (`labs.annotate_replay`, macOS 27+).
    Replay {
        #[command(flatten)]
        output: Output,
    },
    /// The newest captures, newest first.
    List {
        /// How many to show (20 unless given).
        #[arg(long)]
        limit: Option<usize>,
        #[command(flatten)]
        output: Output,
    },
    /// One capture's words, files, and where it went.
    Show {
        /// A bundle name, path, or `latest`.
        capture: String,
        #[command(flatten)]
        output: Output,
    },
    /// Send a capture to an OpenCode session.
    Send {
        /// A bundle name, path, or `latest`.
        capture: String,
        /// A session ID, or for the newest capture a session its route
        /// lists by name. Without it, where its route or last send says.
        #[arg(long)]
        to: Option<String>,
        /// Also save a rule that always sends this capture's site or app
        /// there (the newest routed capture only).
        #[arg(long)]
        always: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Turn the pen on or off while a recording runs (as the capsule's pen),
    /// or start one that draws. Bind it in your desktop's keyboard shortcuts
    /// when it has no shortcuts portal.
    #[cfg(target_os = "linux")]
    Draw {
        #[command(flatten)]
        output: Output,
    },
    /// Take back the newest send while it can be undone (as the capsule's
    /// Undo).
    Undo {
        #[command(flatten)]
        output: Output,
    },
    /// What routing sees for a capture and what it decides (Scrawl's
    /// `route --explain`). Sends nothing.
    Route {
        /// A bundle name, path, or `latest`.
        capture: String,
        /// Accepted for Scrawl's spelling; `route` only ever explains.
        #[arg(long, hide = true)]
        explain: bool,
        /// Ask Jev again now (one call).
        #[arg(long)]
        ask_jev: bool,
        #[command(flatten)]
        output: Output,
    },
}

/// Runs `agent-desk capture`: on macOS through the running Agent-Desk; on Linux the
/// folder reads (`list`, `show`, `route`, `send`) run here, and the
/// recording commands go to the running Agent-Desk's listener; on Windows every
/// command goes to the running Agent-Desk. A build without the capture engine
/// answers `unsupported`.
pub fn capture(command: CaptureCommand) -> u8 {
    use crate::control::{ControlCommand as C, ScrawlCaptureAction as A};
    let (command, output) = match command {
        #[cfg(target_os = "linux")]
        CaptureCommand::Start {
            area: Some(area),
            output,
        } => {
            let [x, y, width, height] = area[..] else {
                return print(
                    ControlReply::error("invalid", "--area takes X,Y,WIDTH,HEIGHT"),
                    output,
                );
            };
            return print(
                crate::linux::service::capture_area([x, y, width, height]),
                output,
            );
        }
        CaptureCommand::Start { output, .. } => (C::ScrawlCapture { action: A::Start }, output),
        CaptureCommand::Stop { output } => (C::ScrawlCapture { action: A::Stop }, output),
        CaptureCommand::Cancel { output } => (C::ScrawlCapture { action: A::Cancel }, output),
        CaptureCommand::Toggle { output } => (C::ScrawlCapture { action: A::Toggle }, output),
        CaptureCommand::Status { output } => (C::ScrawlCapture { action: A::Status }, output),
        CaptureCommand::Replay { output } => (C::ScrawlCapture { action: A::Replay }, output),
        CaptureCommand::List { limit, output } => (C::ScrawlCaptureList { limit }, output),
        CaptureCommand::Show { capture, output } => (C::ScrawlCaptureShow { capture }, output),
        CaptureCommand::Send {
            capture,
            to,
            always,
            output,
        } => (
            C::ScrawlCaptureSend {
                capture,
                to,
                always,
            },
            output,
        ),
        CaptureCommand::Route {
            capture,
            explain: _,
            ask_jev,
            output,
        } => (C::ScrawlCaptureRoute { capture, ask_jev }, output),
        CaptureCommand::Undo { output } => (C::ScrawlCaptureUndo, output),
        #[cfg(target_os = "linux")]
        CaptureCommand::Draw { output } => {
            return print(crate::linux::service::capture_draw(), output);
        }
    };
    #[cfg(not(hex_capture))]
    {
        let _ = command;
        print(
            ControlReply::error("unsupported", "Annotate isn't available on this OS yet"),
            output,
        )
    }
    #[cfg(all(hex_capture, any(target_os = "macos", target_os = "windows")))]
    return send(command, output);
    #[cfg(all(hex_capture, target_os = "linux"))]
    {
        let reply = match command {
            C::ScrawlCaptureList { limit } => crate::capture_host::list_captures(limit),
            C::ScrawlCaptureShow { capture } => crate::capture_host::show_capture(&capture),
            // The running Agent-Desk's router sends, so its Undo can take it back.
            C::ScrawlCaptureSend {
                capture,
                to,
                always,
            } => Ok(crate::linux::service::capture_send(capture, to, always)),
            C::ScrawlCaptureUndo => Ok(crate::linux::service::capture_undo()),
            C::ScrawlCaptureRoute { capture, ask_jev } => {
                crate::capture_host::explain_capture(&capture, ask_jev)
            }
            C::ScrawlCapture { action } => Ok(crate::linux::service::capture(action)),
            _ => unreachable!("only capture commands"),
        };
        print(reply.unwrap_or_else(ControlReply::from), output)
    }
}

pub fn settings(command: SettingsCommand) -> u8 {
    match command {
        SettingsCommand::List { output } => send(ControlCommand::SettingsGet { key: None }, output),
        SettingsCommand::Get { key, output } => {
            send(ControlCommand::SettingsGet { key: Some(key) }, output)
        }
        SettingsCommand::Set { key, value, output } => {
            send(ControlCommand::SettingsSet { key, value }, output)
        }
        SettingsCommand::Apply { file, output } => apply_settings(&file, output),
    }
}

/// How handing settings to a running Agent-Desk went, or why it couldn't wait.
enum Handoff {
    /// The running Agent-Desk took the work, or the lock was free and the work was
    /// written here; holds that command's exit code.
    Done(u8),
    /// On Linux, an Agent-Desk too old for `agent-desk` commands answers on its socket.
    #[cfg(target_os = "linux")]
    OlderHex,
    /// An Agent-Desk held the settings lock and didn't answer before the deadline.
    Deadline,
}

/// `settings apply`'s wait, shared with `model install`: an Agent-Desk that holds the
/// service lock may have read settings.json already, so hand the work to the
/// Agent-Desk that answers (`send`), or write it here while the lock is held
/// (`write`). Only a lock that never answers gives up.
fn hand_off(send: impl FnOnce() -> u8, write: impl FnOnce() -> u8) -> Handoff {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
    loop {
        if running() {
            return Handoff::Done(send());
        }
        // An Agent-Desk that holds the service lock may have read settings.json
        // already; wait for it to answer instead of writing behind it.
        #[cfg(target_os = "linux")]
        let lock = crate::desktop::instance::acquire("service");
        #[cfg(not(target_os = "linux"))]
        let lock: color_eyre::Result<()> = Ok(());
        match lock {
            Ok(_lock) if !running() => return Handoff::Done(write()),
            Ok(_) => {}
            #[cfg(target_os = "linux")]
            Err(_) if crate::linux::service::instance_answers() => return Handoff::OlderHex,
            Err(_) if std::time::Instant::now() >= deadline => return Handoff::Deadline,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

/// `agent-desk settings apply`: the running Agent-Desk saves and adopts the settings, so
/// it never overwrites them from memory; with no Agent-Desk running they are
/// written to settings.json here.
fn apply_settings(file: &std::path::Path, output: Output) -> u8 {
    use std::io::Read;

    let mut text = String::new();
    let read = if file == std::path::Path::new("-") {
        std::io::stdin().read_to_string(&mut text).map(drop)
    } else {
        std::fs::read_to_string(file).map(|read| text = read)
    };
    if let Err(error) = read {
        return fail(&format!("couldn't read {}: {error}", file.display()));
    }
    let values: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(&text) {
        Ok(values) => values,
        Err(error) => return fail(&format!("expected a JSON object of settings: {error}")),
    };
    let handoff = hand_off(
        || {
            send(
                ControlCommand::SettingsApply {
                    values: values.clone(),
                },
                output,
            )
        },
        || print(apply_offline(&values), output),
    );
    match handoff {
        Handoff::Done(code) => code,
        #[cfg(target_os = "linux")]
        Handoff::OlderHex => print(older_hex(), output),
        Handoff::Deadline => print(
            ControlReply::error(
                "busy",
                "Agent-Desk is starting but didn't answer; run `agent-desk settings apply` again",
            ),
            output,
        ),
    }
}

fn apply_offline(values: &serde_json::Map<String, serde_json::Value>) -> ControlReply {
    let result = crate::app_settings::AppSettings::load()
        .map_err(|error| {
            super::ControlError::new("internal", format!("couldn't read settings: {error:#}"))
        })
        .and_then(|before| {
            let change =
                super::settings::apply_all(&before, values, &crate::rune_ui::shortcut_rules())?;
            if !change.changed(&before) {
                return Ok(ControlReply::unchanged(
                    "Every setting already has its value",
                ));
            }
            change.settings.save().map_err(|error| {
                super::ControlError::new("internal", format!("couldn't save: {error:#}"))
            })?;
            Ok(ControlReply::done(change.message))
        });
    result.unwrap_or_else(ControlReply::from)
}

/// `agent-desk history clear`; the confirmation is asked here, before Agent-Desk is.
pub fn clear_history(confirm: bool, output: Output) -> u8 {
    if !confirm {
        return print(
            ControlReply::error(
                "confirmation-required",
                "this deletes every retained dictation; pass --yes to clear history",
            ),
            output,
        );
    }
    send(ControlCommand::HistoryClear, output)
}

pub fn send(command: ControlCommand, output: Output) -> u8 {
    match call(ControlRequest::new(command)) {
        Ok(response) => print(response.reply, output),
        Err(reply) => print(reply, output),
    }
}

/// Every OS reaches the running Agent-Desk through its local API's owner-only
/// discovery file and bearer-authenticated `POST /control`.
fn running() -> bool {
    crate::local_api::running_pid().is_some()
}

fn call(request: ControlRequest) -> Result<ControlResponse, ControlReply> {
    if !running() {
        return Err(absent());
    }
    crate::local_api::call_control(&request)
        .map_err(|error| ControlReply::error("transport", format!("{error:#}")))
}

pub(crate) fn not_running() -> ControlReply {
    ControlReply::error(
        "not-running",
        "Agent-Desk is not running; run `agent-desk start`",
    )
}

/// Why no Agent-Desk answered `/control`: on Linux an older Agent-Desk an update left
/// running answers only on its socket (#915).
fn absent() -> ControlReply {
    #[cfg(target_os = "linux")]
    if crate::linux::service::instance_answers() {
        return older_hex();
    }
    not_running()
}

/// An older Agent-Desk that predates `agent-desk` commands over the local API.
#[cfg(target_os = "linux")]
fn older_hex() -> ControlReply {
    ControlReply::error(
        "outdated",
        "an older Agent-Desk is still running (Agent-Desk was updated) and doesn't take `agent-desk` commands; run `agent-desk restart`",
    )
}

/// `agent-desk restart` with an older Agent-Desk running: restarts its systemd unit into
/// the installed build, then waits until the new Agent-Desk answers.
#[cfg(target_os = "linux")]
fn restart_older_hex() -> ControlReply {
    use std::time::{Duration, Instant};

    if !crate::linux::service::supervised() {
        return ControlReply::error(
            "failed",
            "an older Agent-Desk is running outside its systemd unit; quit it from its window or tray, then run `agent-desk start`",
        );
    }
    if let Err(error) = crate::linux::service::restart() {
        return ControlReply::error("failed", format!("{error:#}"));
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if running() {
            return ControlReply::done("Restarted Agent-Desk");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    ControlReply::done("Restarted Agent-Desk; it isn't answering yet (it may be loading its model)")
}

#[cfg(target_os = "macos")]
fn start(output: Output) -> u8 {
    use std::time::{Duration, Instant};

    if let Some(pid) = crate::local_api::running_pid() {
        return print(
            ControlReply::unchanged(format!("Agent-Desk is already running (pid {pid})")),
            output,
        );
    }
    let arguments = open_arguments(
        installed_bundle().as_deref(),
        crate::app_paths::variant().bundle_id(),
        std::env::vars_os(),
    );
    // Agents (and their guards, this executable) an earlier Agent-Desk left
    // running make macOS refuse to open the app (-600): they end first.
    let _ = crate::assistant::provider::orphans::sweep();
    let open = || {
        std::process::Command::new("/usr/bin/open")
            .args(&arguments)
            .status()
            .is_ok_and(|status| status.success())
    };
    let opened = open() || {
        std::thread::sleep(Duration::from_secs(2));
        open()
    };
    match opened {
        true => {}
        false => {
            return print(
                ControlReply::error("failed", "couldn't open Agent-Desk.app; is it installed?"),
                output,
            );
        }
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if let Some(pid) = crate::local_api::running_pid() {
            return print(
                ControlReply::done(format!("Started Agent-Desk (pid {pid})")),
                output,
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    print(
        ControlReply::done("Opened Agent-Desk; it isn't answering yet (setup may be waiting)"),
        output,
    )
}

/// `agent-desk start` on Linux and Windows: a detached `agent-desk app --hidden`, which
/// forwards to the running Agent-Desk, starts its systemd unit, or becomes Agent-Desk,
/// then waits until it answers.
#[cfg(not(target_os = "macos"))]
fn start(output: Output) -> u8 {
    print(start_reply(), output)
}

#[cfg(not(target_os = "macos"))]
fn start_reply() -> ControlReply {
    use std::time::{Duration, Instant};

    if running() {
        return ControlReply::unchanged("Agent-Desk is already running");
    }
    #[cfg(target_os = "linux")]
    if crate::linux::service::instance_answers() {
        return older_hex();
    }
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => return ControlReply::error("failed", format!("couldn't find hex: {error}")),
    };
    let mut command = std::process::Command::new(executable);
    command
        .args(["app", "--hidden"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "linux")]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: no console, and
        // Ctrl-C in this terminal doesn't reach it.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    if let Err(error) = command.spawn() {
        return ControlReply::error("failed", format!("couldn't start Agent-Desk: {error}"));
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(20) {
        if running() {
            return ControlReply::done("Started Agent-Desk");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    ControlReply::done("Started Agent-Desk; it isn't answering yet (it may be loading its model)")
}

/// After a restart's quit on Linux and Windows: waits for the old Agent-Desk to
/// go, then starts the new one. A Linux unit restarts by itself.
#[cfg(not(target_os = "macos"))]
fn reopen_after_restart() -> ControlReply {
    use std::time::{Duration, Instant};

    #[cfg(target_os = "linux")]
    if crate::linux::service::supervised() {
        return ControlReply::done("Restarting Agent-Desk");
    }
    let started = Instant::now();
    while running() {
        if started.elapsed() > Duration::from_secs(30) {
            return ControlReply::error(
                "failed",
                "Agent-Desk is still finishing its work; run `agent-desk start` once it quits",
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    start_reply()
}

/// `open` arguments for `agent-desk start`: the caller's `AGENT_DESK_*` environment, then
/// the Agent-Desk.app this `agent-desk` belongs to, or the installed app by bundle id.
#[cfg(target_os = "macos")]
fn open_arguments(
    bundle: Option<&std::path::Path>,
    bundle_id: &str,
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<std::ffi::OsString> {
    let mut arguments = crate::desktop::onboarding::open_env_arguments(vars);
    match bundle {
        Some(bundle) => arguments.push(bundle.into()),
        None => arguments.extend(["-b".into(), bundle_id.into()]),
    }
    arguments
}

/// The Agent-Desk.app this `agent-desk` belongs to, when it runs from (or links into) one.
#[cfg(target_os = "macos")]
fn installed_bundle() -> Option<std::path::PathBuf> {
    let executable = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    executable
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(ToOwned::to_owned)
}

fn logs(process: bool, lines: usize, follow: bool) -> u8 {
    use std::io::{BufRead, Seek, SeekFrom};

    let path = match crate::app_paths::logs_dir() {
        Ok(directory) => directory.join(if process {
            "process.log"
        } else {
            "live.ndjson"
        }),
        Err(error) => return fail(&format!("{error:#}")),
    };
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) => return fail(&format!("couldn't read {}: {error}", path.display())),
    };
    let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    // Read at most the last 4 MiB for the tail.
    let start = length.saturating_sub(4 * 1024 * 1024);
    let mut reader = std::io::BufReader::new(&mut file);
    if reader.seek(SeekFrom::Start(start)).is_err() {
        return fail("couldn't read the log");
    }
    let mut tail = std::collections::VecDeque::with_capacity(lines.min(10_000));
    let mut line = String::new();
    let mut first = start > 0;
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if !std::mem::take(&mut first) {
            if tail.len() == lines {
                tail.pop_front();
            }
            if lines > 0 {
                tail.push_back(line.clone());
            }
        }
        line.clear();
    }
    let mut stdout = std::io::stdout().lock();
    for line in tail {
        let _ = stdout.write_all(line.as_bytes());
    }
    let _ = stdout.flush();
    if !follow {
        return 0;
    }
    let mut position = reader.stream_position().unwrap_or(length);
    drop(reader);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let length = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(_) => continue,
        };
        if length < position {
            // Rotated or truncated: start over from the new file.
            match std::fs::File::open(&path) {
                Ok(reopened) => file = reopened,
                Err(_) => continue,
            }
            position = 0;
        }
        if length == position {
            continue;
        }
        if file.seek(SeekFrom::Start(position)).is_err() {
            continue;
        }
        let mut reader = std::io::BufReader::new(&mut file);
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if !line.ends_with('\n') {
                break;
            }
            position += line.len() as u64;
            if stdout.write_all(line.as_bytes()).is_err() {
                return 0;
            }
            line.clear();
        }
        line.clear();
        let _ = stdout.flush();
    }
}

fn fail(message: &str) -> u8 {
    eprintln!("agent-desk: {message}");
    EXIT_REFUSED
}

pub(crate) use human::print;
#[cfg(test)]
use human::*;

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::control::{StatusReport, settings::SettingValue};
    use crate::dictation::vocabulary::VocabularyEntry;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(&ControlReply::done("ok")), 0);
        assert_eq!(exit_code(&ControlReply::unchanged("ok")), 0);
        assert_eq!(exit_code(&ControlReply::error("busy", "x")), EXIT_REFUSED);
        assert_eq!(exit_code(&not_running()), EXIT_NOT_RUNNING);
        assert_eq!(
            exit_code(&ControlReply::error("no-speech", "x")),
            EXIT_NO_SPEECH
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_older_hex_is_not_reported_as_not_running() {
        let reply = older_hex();
        assert_eq!(exit_code(&reply), EXIT_REFUSED);
        assert!(matches!(
            &reply,
            ControlReply::Error { code, message } if code == "outdated" && message.contains("hex restart")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn start_forwards_the_callers_hex_environment() {
        let vars = || {
            [
                ("HOME", "/Users/alex"),
                ("AGENT_DESK_RETAIN_DICTATION_AUDIO", "1"),
                ("AGENT_DESK_ENABLE_LLM", "1"),
            ]
            .map(|(key, value)| (key.into(), value.into()))
        };
        let strings = |arguments: Vec<std::ffi::OsString>| {
            arguments
                .into_iter()
                .map(|argument| argument.into_string().unwrap())
                .collect::<Vec<_>>()
        };
        let env = [
            "--env",
            "AGENT_DESK_ENABLE_LLM=1",
            "--env",
            "AGENT_DESK_RETAIN_DICTATION_AUDIO=1",
        ];
        let bundle = std::path::Path::new("/Applications/Agent-Desk.app");
        assert_eq!(
            strings(open_arguments(Some(bundle), "fheuer.agentdesk", vars())),
            [&env[..], &["/Applications/Agent-Desk.app"]].concat()
        );
        assert_eq!(
            strings(open_arguments(None, "fheuer.agentdesk", vars())),
            [&env[..], &["-b", "fheuer.agentdesk"]].concat()
        );
        assert_eq!(
            strings(open_arguments(None, "fheuer.agentdesk.test", [])),
            ["-b", "fheuer.agentdesk.test"]
        );
    }

    #[test]
    fn one_setting_prints_its_bare_value() {
        let setting = |key: &str, value: Value| SettingValue {
            key: key.into(),
            value,
            kind: "text".into(),
            choices: vec![],
            writable: true,
            about: String::new(),
            available: true,
        };
        assert_eq!(
            human_settings(&[setting("dictation_hotkey", "option".into())]),
            "option\n"
        );
        assert_eq!(human_settings(&[setting("a", Value::Null)]), "none\n");
        let listed = human_settings(&[
            setting("sound_effects", true.into()),
            setting("a", 0.5.into()),
        ]);
        assert_eq!(listed, "sound_effects  true\na              0.5\n");
    }

    #[test]
    fn vocabulary_lists_terms_with_their_spellings() {
        assert!(human_vocabulary(&[]).starts_with("The vocabulary is empty"));
        let entries = [
            VocabularyEntry {
                term: "cappuccino".into(),
                sounds_like: vec!["capuchin".into(), "cap a chino".into()],
            },
            VocabularyEntry {
                term: "Rune".into(),
                sounds_like: vec![],
            },
        ];
        assert_eq!(
            human_vocabulary(&entries),
            "cappuccino  (sounds like capuchin, cap a chino)\nRune\n"
        );
    }

    #[test]
    fn status_reads_for_a_person() {
        let mut report = StatusReport {
            app_version: "2.3.1".into(),
            pid: 42,
            memory_footprint_bytes: Some(412_000_000),
            ..Default::default()
        };
        report.meeting.state = "recording".into();
        report.meeting.title = Some("Design sync".into());
        report.meeting.elapsed_seconds = Some(3723);
        report.features.meetings = true;
        let text = human_status(&report);
        assert!(text.starts_with("Agent-Desk 2.3.1 (pid 42, 412 MB)\n"));
        assert!(text.contains("Meeting      recording “Design sync” for 1:02:03\n"));
        assert!(!text.contains("Labs"));
    }

    #[test]
    fn status_reads_older_replies_that_still_list_labs() {
        let mut json = serde_json::to_value(StatusReport::default()).unwrap();
        assert!(json.get("labs").is_none());
        json.as_object_mut()
            .unwrap()
            .insert("labs".into(), serde_json::json!(["live-text"]));
        let _older: StatusReport = serde_json::from_value(json).unwrap();
    }
}
