//! `hex bench`: Agent-Desk's local benchmark (docs/bench.md). About 25
//! typical tasks as data (`bench/tasks/*.toml`), asked of the running Hex
//! Dev (or, only with `--target live`, Hex) through its owner-only local
//! API, each checked by machine: the answer, the desktop as agent-desktop
//! lists it (read only), files in the run's temp folder under
//! `~/Documents/Hex/bench`, the web addresses and tools the task used.
//! Each run lands in `bench/results/<version>-<timestamp>.json` with a
//! comparison to the run before.
//!
//! The bench never starts Hex, never clicks or types, and never asks
//! anything that would send a message: messages are drafts to the test
//! recipients in `bench/config.toml`, and every task keeps the person's
//! front app unless the run allows otherwise (`--allow-front`).

pub mod check;
pub mod live;
pub mod results;
pub mod run;
#[cfg(test)]
mod run_tests;
pub mod task;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};
use color_eyre::{Result, eyre::eyre};

use self::live::Target;
use self::results::{RunResult, Status};
use self::task::{Config, Lang, Task};

pub const EXIT_WORSE: u8 = 1;
pub const EXIT_INVALID: u8 = 2;
pub const EXIT_NOT_RUNNING: u8 = 3;

#[derive(Subcommand, Debug)]
pub enum BenchCommand {
    /// Ask every task of the running Hex Dev, check the results, and save
    /// them with a comparison to the run before. Exits 1 when the run is
    /// worse than the one before, 2 when a task file breaks the rules, and
    /// 3 when the target isn't running (the bench never starts it).
    Run(RunArgs),
    /// Compare the newest result with the one before it (or FILE with
    /// --against).
    Report {
        /// A result file; the newest by default.
        file: Option<PathBuf>,
        /// The result to compare with; the one before FILE by default.
        #[arg(long)]
        against: Option<PathBuf>,
        /// The bench folder (holding tasks/ and results/); found from here
        /// upward by default.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Print the comparison as JSON.
        #[arg(long)]
        json: bool,
        /// Exit 1 when it is worse than the one before.
        #[arg(long)]
        fail_on_regression: bool,
    },
    /// List the tasks, what they need, and what they check.
    List {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum LangArg {
    De,
    En,
    Both,
}

impl LangArg {
    fn langs(self) -> Vec<Lang> {
        match self {
            Self::De => vec![Lang::De],
            Self::En => vec![Lang::En],
            Self::Both => vec![Lang::De, Lang::En],
        }
    }
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Only tasks whose id matches this regular expression (and the tasks
    /// they follow up on).
    #[arg(long)]
    pub filter: Option<String>,
    /// The Hex to ask. `live` must be named: the bench runs against the
    /// Dev app otherwise.
    #[arg(long, value_enum, default_value = "dev")]
    pub target: Target,
    /// The language the tasks are asked in.
    #[arg(long, value_enum, default_value = "de")]
    pub lang: LangArg,
    /// Check the task files, the config, and every check without asking
    /// anything or reading the desktop.
    #[arg(long)]
    pub dry_run: bool,
    /// Also run tasks that bring an app forward (switch to an app, reveal
    /// in Finder): only when nobody is using the Mac.
    #[arg(long)]
    pub allow_front: bool,
    /// The bench folder (holding tasks/, results/, config.toml); found from
    /// here upward by default.
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Print the result as JSON instead of the table.
    #[arg(long)]
    pub json: bool,
}

/// Runs `command`; the process's exit code.
pub fn run(command: BenchCommand) -> u8 {
    let outcome = match command {
        BenchCommand::Run(args) => run_command(&args),
        BenchCommand::Report {
            file,
            against,
            dir,
            json,
            fail_on_regression,
        } => report(dir, file, against, json, fail_on_regression),
        BenchCommand::List { dir } => list(dir),
    };
    match outcome {
        Ok(code) => code,
        Err(error) => {
            eprintln!("hex bench: {error:#}");
            EXIT_WORSE
        }
    }
}

/// The bench folder: `dir`, or `bench/` here or in a folder above.
pub fn bench_dir(dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = dir {
        return Ok(dir);
    }
    let mut here = std::env::current_dir()?;
    loop {
        let candidate = here.join("bench");
        if candidate.join("tasks").is_dir() {
            return Ok(candidate);
        }
        if !here.pop() {
            return Err(eyre!(
                "no bench/tasks here or above; run it in the repository or pass --dir"
            ));
        }
    }
}

/// `~/Documents/Hex/bench`: every run's temp folders are inside it.
pub fn temp_root() -> Result<PathBuf> {
    Ok(crate::app_paths::document_dir()
        .ok_or_else(|| eyre!("no Documents folder"))?
        .join("Hex")
        .join("bench"))
}

/// Tasks whose id matches `filter`, with the ones they follow up on.
pub fn select(tasks: Vec<Task>, filter: Option<&str>) -> Result<Vec<Task>> {
    let Some(filter) = filter else {
        return Ok(tasks);
    };
    let pattern = regex::Regex::new(filter).map_err(|error| eyre!("--filter: {error}"))?;
    let mut keep: std::collections::BTreeSet<String> = tasks
        .iter()
        .filter(|task| pattern.is_match(&task.id))
        .map(|task| task.id.clone())
        .collect();
    // Follow-ups need the task they continue; take them along.
    loop {
        let more: Vec<String> = tasks
            .iter()
            .filter(|task| keep.contains(&task.id))
            .filter_map(|task| task.after.clone())
            .filter(|after| !keep.contains(after))
            .collect();
        if more.is_empty() {
            break;
        }
        keep.extend(more);
    }
    Ok(tasks
        .into_iter()
        .filter(|task| keep.contains(&task.id))
        .collect())
}

/// The config a run uses: `config.toml`, or for a dry run without one,
/// `config.example.toml`'s sample values.
fn config_for(dir: &Path, dry_run: bool) -> Result<(Config, &'static str)> {
    if let Some(config) = task::load_config(&dir.join("config.toml"))? {
        return Ok((config, "config.toml"));
    }
    if dry_run && let Some(config) = task::load_config(&dir.join("config.example.toml"))? {
        return Ok((config, "config.example.toml (no config.toml)"));
    }
    Ok((Config::new(), "none (tasks that need it are skipped)"))
}

fn run_command(args: &RunArgs) -> Result<u8> {
    let dir = bench_dir(args.dir.clone())?;
    let all = task::load_dir(&dir.join("tasks"))?;
    let (config, config_source) = config_for(&dir, args.dry_run)?;
    let problems = task::validate(&all, Some(&config));
    let tasks = select(all, args.filter.as_deref())?;
    if args.dry_run {
        return Ok(dry_run(&tasks, &config, config_source, &problems, args));
    }
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("{}: {}", problem.task, problem.message);
        }
        eprintln!("hex bench: fix the task files first (`hex bench run --dry-run`)");
        return Ok(EXIT_INVALID);
    }
    let support = args.target.support_dir()?;
    let mut app = live::LiveApp::new(support);
    let version = match run::App::version(&mut app) {
        Ok(version) => version,
        Err(error) => {
            eprintln!(
                "hex bench: {} isn't running ({error:#}). The bench never starts it: open it \
                 yourself (starting Hex Dev quits Hex), or run scripts/self-improve.sh.",
                args.target.app_name()
            );
            if args.target == Target::Dev {
                eprintln!("It never runs against Hex unless you pass --target live.");
            }
            return Ok(EXIT_NOT_RUNNING);
        }
    };
    let started_ms = now_ms();
    let run_id = timestamp(started_ms);
    let run_dir = temp_root()?.join(&run_id);
    std::fs::create_dir_all(&run_dir)?;
    let options = run::Options {
        langs: args.lang.langs(),
        allow_front: args.allow_front,
        poll_ms: 250,
        run_dir,
        run_id: run_id.clone(),
        background_wait_ms: 30_000,
    };
    eprintln!(
        "hex bench: {} tasks against {} {version} (run {run_id})",
        tasks.len() * options.langs.len(),
        args.target.app_name()
    );
    let mut desktop = live::MacDesktop::new();
    let results = run::run(
        &tasks,
        &config,
        &mut app,
        &mut desktop,
        &run::SystemClock,
        &options,
        &mut |result| {
            eprintln!(
                "  {:<7} {:<28} {} {}",
                result.status.label(),
                result.id,
                result.lang.code(),
                match result.status {
                    Status::Passed => result
                        .total_ms
                        .map_or_else(String::new, |ms| format!("{ms} ms")),
                    _ => result.reason.clone(),
                }
            );
        },
    );
    let result = RunResult {
        schema: results::SCHEMA,
        version,
        target: args.target.name().into(),
        run_id,
        started_ms,
        finished_ms: now_ms(),
        filter: args.filter.clone(),
        tasks: results,
    };
    let results_dir = dir.join("results");
    let before = previous(&results_dir, &result);
    let path = results::save(&results_dir, &result)?;
    let comparison = results::compare(&result, before.as_ref());
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", results::table(&result, before.as_ref(), &comparison));
        println!("Saved {}", path.display());
    }
    Ok(if comparison.worse() { EXIT_WORSE } else { 0 })
}

/// The newest earlier result of the same target.
fn previous(dir: &Path, now: &RunResult) -> Option<RunResult> {
    results::load_all(dir)
        .into_iter()
        .map(|(_, result)| result)
        .rfind(|result| result.target == now.target && result.started_ms < now.started_ms)
}

fn dry_run(
    tasks: &[Task],
    config: &Config,
    config_source: &str,
    problems: &[task::Problem],
    args: &RunArgs,
) -> u8 {
    println!(
        "Dry run: {} tasks, config from {config_source}; nothing is asked and the desktop isn't read.\n",
        tasks.len()
    );
    let mut values = config.clone();
    values.insert(
        "temp_dir".into(),
        "~/Documents/Hex/bench/<run>/<task>".into(),
    );
    values.insert("run".into(), "<run>".into());
    for task in tasks {
        let missing = task.missing(config);
        let state = if problems.iter().any(|problem| problem.task == task.id) {
            "INVALID".to_owned()
        } else if !missing.is_empty() {
            format!("skips without {}", missing.join(", "))
        } else if task.safety.front == task::Front::MayChange && !args.allow_front {
            "skips without --allow-front".to_owned()
        } else {
            "ready".to_owned()
        };
        println!(
            "{:<28} {:<5} {:>3} checks  {state}",
            task.id,
            format!("{:?}", task.lane).to_lowercase(),
            task.checks.len() + task.precondition.len()
        );
        for lang in args.lang.langs() {
            let text = task::fill(task.text.get(lang), &values, false)
                .unwrap_or_else(|key| format!("(needs {{{key}}}) {}", task.text.get(lang)));
            println!("    {}: {text}", lang.code());
        }
    }
    if problems.is_empty() {
        println!("\nAll task files keep the bench's rules.");
        0
    } else {
        println!();
        for problem in problems {
            println!("INVALID {}: {}", problem.task, problem.message);
        }
        EXIT_INVALID
    }
}

fn report(
    dir: Option<PathBuf>,
    file: Option<PathBuf>,
    against: Option<PathBuf>,
    json: bool,
    fail_on_regression: bool,
) -> Result<u8> {
    let results_dir = bench_dir(dir)?.join("results");
    let now = match file {
        Some(file) => results::load(&file)?,
        None => results::load_all(&results_dir)
            .pop()
            .map(|(_, result)| result)
            .ok_or_else(|| eyre!("no results in {} yet", results_dir.display()))?,
    };
    let before = match against {
        Some(file) => Some(results::load(&file)?),
        None => previous(&results_dir, &now),
    };
    let comparison = results::compare(&now, before.as_ref());
    if json {
        println!("{}", serde_json::to_string_pretty(&comparison)?);
    } else {
        print!("{}", results::table(&now, before.as_ref(), &comparison));
    }
    Ok(if fail_on_regression && comparison.worse() {
        EXIT_WORSE
    } else {
        0
    })
}

fn list(dir: Option<PathBuf>) -> Result<u8> {
    let tasks = task::load_dir(&bench_dir(dir)?.join("tasks"))?;
    for task in &tasks {
        let needs = task.needs();
        println!(
            "{:<28} {:<5} {:>4}s  {}{}",
            task.id,
            format!("{:?}", task.lane).to_lowercase(),
            task.timeout_s,
            task.title,
            if needs.is_empty() {
                String::new()
            } else {
                format!(
                    "  (needs {})",
                    needs.into_iter().collect::<Vec<_>>().join(", ")
                )
            }
        );
        for check in &task.checks {
            println!("{:>36} {}", "·", check.label());
        }
    }
    Ok(0)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis() as u64)
        .unwrap_or_default()
}

/// `YYYYMMDD-HHMMSS` (UTC) for Unix ms.
pub fn timestamp(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// `text` cut to `limit` characters, with an ellipsis.
pub fn shorten(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
    out.push('…');
    out
}
