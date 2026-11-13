//! A job keeps going until its goal is met, as a person would: it writes
//! what "done" means (`## Done when` in `plan.md`), and when it answers, an
//! independent check (a fresh session of the same backend with the same
//! tools, told to change nothing) looks for real evidence of every
//! criterion. Gaps go back to the job as its next instruction; only a
//! verified result (or, when it is stuck, an honest account of what is
//! missing) reaches the person. Nothing of this shows: the check's steps
//! and the job's in-between answers are neither shown nor spoken.
//!
//! The rounds are guarded: no progress on the same gaps for a few rounds
//! makes the job change its approach, then ask the person for what only
//! they can give (`HEX_QUESTION`), and in the end wrap up with what is
//! missing. The job's time budget (`job_budget_minutes`, up to seven days)
//! bounds it all.
//!
//! OWNER: brain.

/// Plan steps up to which a job counts as trivial: its own answer stands
/// without an independent check (one check took 16 s at the median).
pub const TRIVIAL_STEPS: usize = 2;

/// Whether a job with `steps` plan steps is worth an independent check: a
/// real piece of work is; a one- or two-step job (or one without a plan)
/// isn't.
pub fn worth_checking(steps: usize) -> bool {
    steps > TRIVIAL_STEPS
}

/// The check's verdict line when every criterion is met.
pub const VERIFIED: &str = "HEX_VERIFIED";
/// The check's verdict when something is missing; the gaps follow.
pub const NOT_DONE: &str = "HEX_NOT_DONE:";
/// The job's last check, in its workspace, for the status Hex tells on
/// request (`service::status`): `verified`, or the gaps.
pub const CHECK_FILE: &str = ".hex-check.md";

/// Rounds of work and checks one run may take before it wraps up anyway.
pub const MAX_ROUNDS: u32 = 40;
/// Rounds without progress after which the job is told to change approach.
pub const STALE_RETHINK: u32 = 2;
/// … after which it should ask the person for what only they can give.
pub const STALE_ASK: u32 = 5;
/// … after which it stops and says what is missing.
pub const STALE_GIVE_UP: u32 = 8;

/// What the check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    NotDone(String),
    /// No verdict (the check failed or said neither): the job's answer
    /// stands.
    Unclear,
}

/// The verdict in the check's reply: its last `HEX_VERIFIED` or
/// `HEX_NOT_DONE:` line (the gaps are the rest of that line and the lines
/// after it).
pub fn verdict(text: &str) -> Verdict {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate().rev() {
        let line = line.trim().trim_start_matches(['*', '`', '#', ' ']);
        if let Some(rest) = line.strip_prefix(NOT_DONE) {
            let rest = rest.trim().trim_start_matches(['*', '`', ' ']);
            let mut gaps = vec![rest.trim().to_owned()];
            gaps.extend(lines[index + 1..].iter().map(|line| line.trim().to_owned()));
            let gaps = gaps
                .into_iter()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            return if gaps.is_empty() {
                Verdict::NotDone("Some criteria are not met yet.".into())
            } else {
                Verdict::NotDone(gaps)
            };
        }
        if line.starts_with(VERIFIED) {
            return Verdict::Verified;
        }
    }
    Verdict::Unclear
}

/// What the job does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// The answer stands: tell the person.
    Done,
    /// Go on with this instruction.
    Continue(String),
    /// One last turn with this instruction, whose answer then stands.
    WrapUp(String),
}

/// The rounds so far.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub rounds: u32,
    /// Rounds in a row whose check found the same gaps.
    pub stale: u32,
    last_gaps: Option<String>,
}

impl Progress {
    /// The next step after a check found `verdict`.
    pub fn next(&mut self, verdict: Verdict) -> Next {
        let gaps = match verdict {
            Verdict::Verified | Verdict::Unclear => return Next::Done,
            Verdict::NotDone(gaps) => gaps,
        };
        self.rounds += 1;
        let same = self
            .last_gaps
            .as_deref()
            .is_some_and(|last| normal(last) == normal(&gaps));
        self.stale = if same { self.stale + 1 } else { 0 };
        self.last_gaps = Some(gaps.clone());
        if self.rounds >= MAX_ROUNDS || self.stale >= STALE_GIVE_UP {
            return Next::WrapUp(format!(
                "Stop working on this now. An independent check still found these gaps:\n{gaps}\n\
                 Answer the person as usual: what is done, with its links and files, and plainly what is still missing and why. Don't mention the check."
            ));
        }
        let mut prompt = format!(
            "An independent check of your work against the plan's “Done when” found these gaps:\n{gaps}\n\
             Keep working until every criterion is met, with real evidence (open it, run it, look at it). Update plan.md and Nachweis.md as you go. Don't ask the person about anything you can decide: choose the sensible default and note it. When everything is met, answer as before with your short summary. Don't mention the check to the person."
        );
        if self.stale >= STALE_RETHINK {
            prompt.push_str(
                "\nThe same gaps came back after your last attempts: your approach isn't working. Step back, find out why, and try a different way.",
            );
        }
        if self.stale >= STALE_ASK {
            prompt.push_str(
                "\nIf what's missing is something only the person can give (an account, a password, a decision, data you can't find), ask now: start your reply with the line `HEX_QUESTION:` and one or two speakable sentences.",
            );
        }
        Next::Continue(prompt)
    }
}

/// Gaps compared loosely: case, punctuation, and spacing aside.
fn normal(gaps: &str) -> String {
    gaps.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// What a job writes about its goal, appended to its rules.
pub const DONE_WHEN_RULES: &str = "Goal: in plan.md, under the checklist, add a section `## Done when` with three to eight concrete criteria that prove the request is fulfilled, each checkable with real evidence (a URL that answers and shows X, a file with Y, a message visible in the sent folder, a page that looks like the reference by hex_compare_images on screenshots at the same window size, tests that pass). Derive them from the person's own words, including every deliverable they named (links to give, people to send to, where to publish). The last criterion is always: Nachweis.md proves each of the others (links, screenshots, diff images, sent-message screenshots). Keep working until each one is met, check them yourself before you answer, and never answer with \"done\" while one is open. Long jobs are fine: you may work for hours or days, unattended: the person may be away.\n";

/// What the check is told, as its message.
pub fn check_prompt(request: &str, answer: &str) -> String {
    format!(
        "You are now an independent checker, not the worker: ignore the rules about planning, asking, and answering the person. Change nothing (no edits, sends, posts, or deploys); only look.\n\
         The person's request was: “{request}”\n\
         The worker says it is done: “{answer}”\n\
         Read plan.md in your current folder, especially `## Done when` (if it is missing, derive the criteria from the request yourself). Check every criterion with real evidence using your tools: open URLs and pages, run builds or tests, list files, take screenshots and compare them (hex_compare_images compares two screenshots at the same viewport size pixel by pixel; pass the threshold the request implies, 0.5 for pixel-identical), look at the sent message in its conversation or the published result, and read Nachweis.md and check that its proofs exist and show what they claim. Don't trust the worker's word.\n\
         Then reply with exactly one verdict as your last line(s): `{VERIFIED}` when every criterion is met, or `{NOT_DONE}` followed by each unmet criterion and what you saw, one per line, concrete enough to act on."
    )
}

/// The check's record in the job's workspace (`CHECK_FILE`).
pub fn check_record(verdict: &Verdict) -> Option<String> {
    match verdict {
        Verdict::Verified => Some("verified\n".into()),
        Verdict::NotDone(gaps) => Some(format!("{gaps}\n")),
        Verdict::Unclear => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdict_is_read_from_the_last_marker() {
        assert_eq!(
            verdict("Looked at everything.\nHEX_VERIFIED"),
            Verdict::Verified
        );
        assert_eq!(
            verdict("Checked.\n**HEX_NOT_DONE:** the URL returns 502\n- no GitHub remote\n"),
            Verdict::NotDone("the URL returns 502\n- no GitHub remote".into())
        );
        assert_eq!(
            verdict("HEX_NOT_DONE: x\nlater I saw it works\nHEX_VERIFIED"),
            Verdict::Verified
        );
        assert_eq!(verdict("I think it's fine."), Verdict::Unclear);
        assert!(matches!(verdict("HEX_NOT_DONE:"), Verdict::NotDone(_)));
    }

    #[test]
    fn a_verified_job_is_done_at_once_and_one_on_its_third_round() {
        let mut progress = Progress::default();
        assert_eq!(progress.next(Verdict::Verified), Next::Done);
        let mut progress = Progress::default();
        assert!(matches!(
            progress.next(Verdict::NotDone("no deploy".into())),
            Next::Continue(_)
        ));
        assert!(matches!(
            progress.next(Verdict::NotDone("site looks different".into())),
            Next::Continue(_)
        ));
        assert_eq!(progress.next(Verdict::Verified), Next::Done);
        assert_eq!(progress.rounds, 2);
    }

    #[test]
    fn no_progress_rethinks_then_asks_then_wraps_up() {
        let mut progress = Progress::default();
        let gaps = || Verdict::NotDone("The URL returns 502.".into());
        let mut prompts = Vec::new();
        loop {
            match progress.next(gaps()) {
                Next::Continue(prompt) => prompts.push(prompt),
                Next::WrapUp(prompt) => {
                    prompts.push(prompt);
                    break;
                }
                Next::Done => panic!("never verified"),
            }
        }
        assert!(!prompts[0].contains("different way"));
        assert!(prompts[usize::try_from(STALE_RETHINK).unwrap()].contains("different way"));
        assert!(!prompts[usize::try_from(STALE_RETHINK).unwrap()].contains("HEX_QUESTION"));
        assert!(prompts[usize::try_from(STALE_ASK).unwrap()].contains("HEX_QUESTION"));
        assert!(prompts.last().unwrap().contains("Stop working"));
        assert_eq!(progress.stale, STALE_GIVE_UP);
        // Spacing and case don't make gaps new.
        let mut progress = Progress::default();
        progress.next(Verdict::NotDone("URL returns 502".into()));
        progress.next(Verdict::NotDone("url  returns 502.".into()));
        assert_eq!(progress.stale, 1);
    }

    #[test]
    fn rounds_are_capped_even_with_new_gaps() {
        let mut progress = Progress::default();
        for round in 1..MAX_ROUNDS {
            assert!(matches!(
                progress.next(Verdict::NotDone(format!("gap {round}"))),
                Next::Continue(_)
            ));
        }
        assert!(matches!(
            progress.next(Verdict::NotDone("gap last".into())),
            Next::WrapUp(_)
        ));
    }

    #[test]
    fn only_real_work_is_checked() {
        assert!(!super::worth_checking(0));
        assert!(!super::worth_checking(2));
        assert!(super::worth_checking(3));
    }

    #[test]
    fn the_goal_and_its_check_ask_for_proof_in_the_evidence_file() {
        assert!(DONE_WHEN_RULES.contains("Nachweis.md proves each of the others"));
        let check = check_prompt("Rebuild xyz.com pixel-perfect", "Done.");
        assert!(check.contains("read Nachweis.md"), "{check}");
        assert!(check.contains("threshold"), "{check}");
        let mut progress = Progress::default();
        let Next::Continue(prompt) = progress.next(Verdict::NotDone("no repo".into())) else {
            panic!("a job not done goes on");
        };
        assert!(prompt.contains("Don't ask the person about anything you can decide"));
    }
}
