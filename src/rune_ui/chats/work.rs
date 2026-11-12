//! A job of any size, as the transcript shows it: its plan as a checklist
//! at the top (the current step, done of total, time so far; folds to one
//! line), the question it waits on with an answer field (and a sign-in
//! button when it needs one), and once it's done, its folder with the
//! files it made and the links its answer names.

use rune::{El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Spinner, TextInput};

use super::super::controls::{DESCRIPTION, Paint, PushSize};
use super::super::list_detail::{FOLDED, UNFOLDED};
use super::super::lists::small_button;
use super::super::onboarding_kit::arrives;
use super::{ChatsPane, Job, Ui};
use crate::assistant::{Conversation, ConversationId, Origin, PlanStep, Request, Role};
use crate::desktop::i18n::{tr, tr_fill};

static FOLDER_ICON: &rune::Icon = &rune_theme_opencode::icons::FOLDER;
static FILE_ICON: &rune::Icon = &rune_theme_opencode::icons::FILE_DOCUMENT;
static QUESTION_ICON: &rune::Icon = &rune_theme_opencode::icons::HELP;
static DONE_ICON: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_CHECK;

/// The answer field's key, focused when a question arrives.
pub(super) const ANSWER_FIELD: &str = "question-answer";

/// File kinds a job's folder lists.
const WORTH_A_ROW: [&str; 10] = [
    "xlsx", "csv", "md", "docx", "pdf", "txt", "json", "numbers", "pages", "html",
];

pub(super) fn worth_a_row(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| WORTH_A_ROW.contains(&ext.to_lowercase().as_str()))
}

/// Words of `text` that may be paths or links, without their quoting.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | '<' | '>' | '[' | ']'))
        .map(|word| word.trim_end_matches(['.', ',', ';', ':', '"', '\'', '”', '’', '!']))
        .filter(|word| !word.is_empty())
}

/// The folder a finished job's answer names: the first absolute or
/// home-relative path, or the folder of the first file it names. Job
/// folders are named after their request, with spaces ("2026-11-12 Hex
/// selbst testen"), so the longest run of words from the path's start
/// that is a folder on this Mac wins over the first word alone.
pub(super) fn folder_in(text: &str) -> Option<String> {
    folder_in_with(text, &|path| {
        std::path::Path::new(&expand_home(path)).is_dir()
    })
}

fn expand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => crate::app_paths::home_dir()
            .map(|home| home.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned()),
        None => path.to_owned(),
    }
}

/// [`folder_in`] with `is_dir` telling which paths are folders.
fn folder_in_with(text: &str, is_dir: &dyn Fn(&str) -> bool) -> Option<String> {
    let starts_path = |word: &str| {
        word.starts_with("~/")
            || (word.starts_with('/') && word.len() > 1 && word[1..].contains('/'))
    };
    // The longest existing folder from the path's first word on, spaces
    // included (up to the end of its line).
    for line in text.lines() {
        let words: Vec<&str> = line.split(' ').collect();
        if let Some(start) = words.iter().position(|word| {
            starts_path(word.trim_start_matches(['(', '`', '"', '\u{201e}', '\u{201c}']))
        }) {
            for end in (start + 1..=words.len()).rev() {
                let candidate = words[start..end]
                    .join(" ")
                    .trim_start_matches(['(', '`', '"', '\u{201e}', '\u{201c}'])
                    .trim_end_matches(['.', ',', ';', ':', ')', '`', '"', '\u{201c}', '\u{201d}'])
                    .trim_end_matches('/')
                    .to_owned();
                if is_dir(&candidate) {
                    return Some(candidate + "/");
                }
            }
            break;
        }
    }
    let path = words(text).find(|word| starts_path(word))?;
    if path.ends_with('/') {
        return Some(path.trim_end_matches('/').to_owned() + "/");
    }
    let last = path.rsplit('/').next().unwrap_or_default();
    if last.contains('.') {
        let parent = &path[..path.len() - last.len()];
        return (!parent.is_empty()).then(|| parent.to_owned());
    }
    Some(format!("{path}/"))
}

/// The links an answer names that are its results (a Google Sheet or Doc
/// the job made), once each. Sources it cites stay links in the text: a
/// button for every search hit says nothing.
pub(super) fn links_in(text: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    for word in words(text) {
        let word = word.trim_end_matches(['.', ',', ';', ')', ']']);
        let word = word.rsplit("](").next().unwrap_or(word);
        if (word.starts_with("https://") || word.starts_with("http://"))
            && word.contains("docs.google.com/")
            && !links.iter().any(|known| known == word)
        {
            links.push(word.to_owned());
        }
    }
    links
}

/// A link's button: what it is, when that's known.
fn link_label(link: &str) -> String {
    let host = link
        .split("://")
        .nth(1)
        .unwrap_or(link)
        .split('/')
        .next()
        .unwrap_or_default();
    if link.contains("docs.google.com/spreadsheets") {
        tr("Open Google Sheet").to_owned()
    } else if link.contains("docs.google.com/document") {
        tr("Open Google Doc").to_owned()
    } else {
        tr_fill("Open {}", host.trim_start_matches("www."))
    }
}

/// The last answer's text in `conversation`.
fn last_answer(conversation: &Conversation) -> Option<String> {
    conversation
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(crate::assistant::Message::text)
}

/// The job folder a stored conversation's last answer names.
pub(super) fn folder_of(conversation: &Conversation) -> Option<String> {
    folder_in(&last_answer(conversation)?)
}

impl ChatsPane {
    /// Answers the question conversation `id` waits on: sent into it, and
    /// shown under the question.
    pub(super) fn answer_question(&mut self, id: &str, words: String) -> bool {
        let Some(question) = self.questions.remove(id) else {
            return false;
        };
        let origin = self
            .turns
            .get(id)
            .map(|turn| turn.origin)
            .or_else(|| self.task(id).map(|task| task.origin))
            .unwrap_or(Origin::Chat);
        self.source.submit(Request {
            conversation: Some(id.to_owned()),
            text: words.clone(),
            origin,
            context: crate::assistant::Context::default(),
        });
        let title = self
            .task(id)
            .map(|task| task.title.clone())
            .unwrap_or_default();
        self.turns
            .entry(id.to_owned())
            .or_insert_with(|| super::Turn::new(title, origin, super::Phase::Working))
            .answers
            .push((question, words));
        self.answer.clear();
        self.follow.to_end = true;
        true
    }
}

/// The plan's card over a running job's transcript: done of total and the
/// time so far, the steps under it unless folded.
pub(super) fn plan_card(ui: &Ui, p: Paint, id: &str) -> Option<El> {
    let pane = &ui.chats;
    let steps = pane.plans.get(id).filter(|steps| !steps.is_empty())?;
    let done = steps.iter().filter(|step| step.done).count();
    let current = steps.iter().position(|step| !step.done);
    let folded = pane.plan_folded.iter().any(|folded| folded == id);
    let elapsed = pane.turns.get(id).map(|turn| {
        let seconds = crate::history::now_ms().saturating_sub(turn.started_ms) / 1_000;
        match seconds {
            0..=3_599 => format!("{}:{:02}", seconds / 60, seconds % 60),
            _ => format!(
                "{}:{:02}:{:02}",
                seconds / 3_600,
                seconds / 60 % 60,
                seconds % 60
            ),
        }
    });
    // Folded, the line names the step it's on.
    let now = match current {
        Some(index) if folded => steps[index].title.clone(),
        Some(_) => String::new(),
        None => tr("Wrapping up").to_owned(),
    };
    let toggle = id.to_owned();
    let k = p.k;
    let head = row()
        .key("plan-head")
        .role("button")
        .label(tr("Plan"))
        .expanded(!folded)
        .focusable(true)
        .focus_ring(2.0, 0.0, k.c.border_focus)
        .h(32.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .on_press(move |ui: &mut Ui| {
            let folded = &mut ui.chats.plan_folded;
            match folded.iter().position(|id| *id == toggle) {
                Some(index) => {
                    folded.remove(index);
                }
                None => folded.push(toggle.clone()),
            }
        })
        .child(
            icon(if folded { &FOLDED } else { &UNFOLDED }, 14.0)
                .key("chevron")
                .color(k.c.icon_faint)
                .a11y_hidden(),
        )
        .child(
            text(tr("Plan"))
                .key("title")
                .font(k.ty.small_medium)
                .color(p.text()),
        )
        .child(
            text(tr_fill("{} done", &format!("{done} / {}", steps.len())))
                .key("count")
                .font(k.ty.small)
                .color(p.muted()),
        )
        .child(
            text(now)
                .key("now")
                .font(k.ty.small)
                .color(p.muted())
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .children(elapsed.map(|elapsed| {
            text(elapsed)
                .key("elapsed")
                .font(k.ty.small)
                .color(p.faint())
        }));
    let mut card = col()
        .key("plan")
        .role("group")
        .label(tr("Plan"))
        .mr(12.0)
        .mb(6.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(p.hair, k.c.border_weak)
        .clip_soft(0.0)
        .transition(arrives())
        .child(head);
    if !folded {
        let mut list = col().key("steps").role("list").px(12.0).pb(10.0).gap(4.0);
        for (index, step) in steps.iter().enumerate() {
            list = list.child(plan_step(p, index, step, Some(index) == current));
        }
        card = card.child(list);
    }
    Some(card)
}

fn plan_step(p: Paint, index: usize, step: &PlanStep, current: bool) -> El {
    let mark = if step.done {
        icon(DONE_ICON, 14.0).color(p.success())
    } else if current {
        Spinner::new().size(12.0).color(p.accent()).build(p.k)
    } else {
        el().size(10.0, 10.0)
            .radius(5.0)
            .border(1.25, p.k.c.border_strong)
    };
    row()
        .key(format!("step{index}"))
        .role("listitem")
        .label(step.title.clone())
        .checked(step.done)
        .gap(8.0)
        .min_h(20.0)
        .center_y()
        .child(
            el().key("mark")
                .size(14.0, 14.0)
                .center()
                .child(mark)
                .a11y_hidden(),
        )
        .child(
            text(step.title.clone())
                .key("title")
                .font(if current {
                    p.k.ty.small_medium
                } else {
                    p.k.ty.small
                })
                .color(if step.done { p.muted() } else { p.text() })
                .wrap()
                .grow(1.0)
                .shrink()
                .a11y_hidden(),
        )
}

/// The site a question asks to sign in to, when it asks for one: Google
/// for Gmail, Sheets, and Drive, else whatever page the person picks.
fn sign_in_for(question: &str) -> Option<Option<&'static str>> {
    let lower = question.to_lowercase();
    if !(lower.contains("sign in") || lower.contains("log in") || lower.contains("signed in")) {
        return None;
    }
    let google = ["google", "gmail", "sheets", "drive"]
        .iter()
        .any(|word| lower.contains(word));
    Some(google.then_some("https://accounts.google.com"))
}

/// The question a job waits on: highlighted, with its answer field (and
/// Sign in to Google… when that's what it needs).
pub(super) fn question_block(ui: &Ui, p: Paint, id: &str) -> Option<El> {
    let pane = &ui.chats;
    let question = pane.questions.get(id)?;
    let k = p.k;
    let answer_id = id.to_owned();
    let submit_id = id.to_owned();
    let field = TextInput::new(&pane.answer)
        .placeholder(tr("Your answer…"))
        .label(tr("Answer"))
        .width(None)
        .on_edit(|ui: &mut Ui| &mut ui.chats.answer)
        .on_submit(move |ui: &mut Ui, _| {
            let words = ui.chats.answer.text.trim().to_owned();
            if !words.is_empty() {
                ui.chats.answer_question(&submit_id, words);
            }
        })
        .build(k)
        .key(ANSWER_FIELD)
        .grow(1.0)
        .shrink();
    let send = Button::new(tr("Answer"))
        .variant(ButtonVariant::Contrast)
        .on_press(move |ui: &mut Ui| {
            let words = ui.chats.answer.text.trim().to_owned();
            if !words.is_empty() {
                ui.chats.answer_question(&answer_id, words);
            }
        })
        .build_small_push(k)
        .key("send-answer");
    let sign_in = sign_in_for(question).map(|url| {
        let signing_in = pane.settings.signing_in;
        let label = match (signing_in, url) {
            (true, _) => tr("Done signing in"),
            (false, Some(_)) => tr("Sign in to Google…"),
            (false, None) => tr("Sign in…"),
        };
        small_button(p, "question-sign-in", label, move |ui: &mut Ui, _| {
            if ui.chats.settings.signing_in {
                ui.chats.jobs.run(Job::FinishSignIn);
            } else {
                ui.chats.settings.sign_in_error = None;
                ui.chats.jobs.run(Job::SignIn(url.map(str::to_owned)));
            }
        })
    });
    Some(
        col()
            .key("question")
            .role("group")
            .label(tr("It asks"))
            .gap(10.0)
            .px(12.0)
            .py(12.0)
            .radius(k.radius.lg)
            .fill(k.c.layer_01)
            .border(1.0, k.c.info.border)
            .transition(arrives())
            .child(
                row()
                    .key("asks")
                    .gap(8.0)
                    .items_start()
                    .child(
                        icon(QUESTION_ICON, 15.0)
                            .key("icon")
                            .color(p.accent())
                            .mt(2.0)
                            .a11y_hidden(),
                    )
                    .child(
                        rune_kit::Markdown::new(question.clone())
                            .selectable(true)
                            .build(k)
                            .key("words")
                            .grow(1.0)
                            .shrink(),
                    ),
            )
            .child(
                row()
                    .key("reply")
                    .gap(8.0)
                    .center_y()
                    .child(field)
                    .children(sign_in)
                    .child(send),
            ),
    )
}

/// A question it asked while it worked, and the person's answer.
pub(super) fn answered(p: Paint, index: usize, question: &str, answer: &str) -> El {
    col()
        .key(format!("answered{index}"))
        .gap(8.0)
        .child(
            row()
                .key("asked")
                .gap(8.0)
                .items_start()
                .child(
                    icon(QUESTION_ICON, 13.0)
                        .key("icon")
                        .color(p.k.c.icon_faint)
                        .mt(2.0)
                        .a11y_hidden(),
                )
                .child(
                    text(question.to_owned())
                        .key("words")
                        .font(DESCRIPTION)
                        .color(p.muted())
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                ),
        )
        .child(super::transcript::bubble(p, "answer", answer))
}

/// A finished job's results under its answer: its folder with the files
/// it made, and the links its answer names.
pub(super) fn results(ui: &Ui, p: Paint, conversation: &Conversation) -> Option<El> {
    let answer = last_answer(conversation)?;
    let folder = folder_in(&answer);
    let links = links_in(&answer);
    if folder.is_none() && links.is_empty() {
        return None;
    }
    let k = p.k;
    let mut block = col()
        .key("results")
        .role("group")
        .label(tr("Results"))
        .gap(2.0)
        .py(6.0)
        .radius(k.radius.lg)
        .fill(k.c.layer_01)
        .border(p.hair, k.c.border_weak)
        .transition(arrives());
    if let Some(folder) = folder {
        let open = folder.clone();
        block = block.child(result_row(
            p,
            "folder",
            FOLDER_ICON,
            &folder,
            Some((
                tr("Open folder"),
                Box::new(move |ui: &mut Ui| ui.chats.source.open(&open, false)),
            )),
            true,
        ));
        let files = ui.chats.folders.get(&folder).cloned().unwrap_or_default();
        for (index, name) in files.iter().enumerate() {
            let path = format!("{folder}{name}");
            let look = path.clone();
            block = block.child(
                result_row(p, &format!("file{index}"), FILE_ICON, name, None, false)
                    .role("button")
                    .label(tr_fill("Quick Look {}", name))
                    .focusable(true)
                    .hover(k.c.hover)
                    .on_press(move |ui: &mut Ui| ui.chats.source.open(&look, true)),
            );
        }
    }
    if !links.is_empty() {
        let mut buttons = row().key("links").px(12.0).py(6.0).gap(8.0).flex_wrap();
        for (index, link) in links.iter().take(4).enumerate() {
            let open = link.clone();
            buttons = buttons.child(
                Button::new(link_label(link))
                    .variant(ButtonVariant::Neutral)
                    .on_press(move |ui: &mut Ui| ui.chats.source.open(&open, false))
                    .build_small_push(k)
                    .key(format!("link{index}"))
                    .help(link.clone()),
            );
        }
        block = block.child(buttons);
    }
    Some(block)
}

type Press = Box<dyn Fn(&mut Ui)>;

fn result_row(
    p: Paint,
    key: &str,
    glyph: &'static rune::Icon,
    title: &str,
    action: Option<(&str, Press)>,
    strong: bool,
) -> El {
    let row_el = row()
        .key(key.to_owned())
        .h(30.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(p.k.radius.md)
        .child(
            icon(glyph, 14.0)
                .key("icon")
                .color(p.k.c.icon_base)
                .a11y_hidden(),
        )
        .child(
            text(title.to_owned())
                .key("title")
                .font(if strong {
                    p.k.ty.small_medium
                } else {
                    p.k.ty.small
                })
                .color(p.text())
                .grow(1.0)
                .shrink()
                .truncate(),
        );
    match action {
        Some((label, press)) => row_el.child(
            Button::new(label)
                .variant(ButtonVariant::Neutral)
                .on_press(move |ui: &mut Ui| press(ui))
                .build_small_push(p.k)
                .key("open"),
        ),
        None => row_el,
    }
}

/// Focuses the answer field once a question arrives in the shown
/// conversation.
pub(super) fn poll(ui: &mut Ui, tx: &mut Tx) {
    if std::mem::take(&mut ui.chats.focus_answer) {
        tx.focus(ANSWER_FIELD, false);
    }
    let running_plan = ui
        .chats
        .selected
        .as_ref()
        .is_some_and(|id| ui.chats.plans.contains_key(id) && ui.chats.running(id));
    if running_plan {
        // The elapsed time ticks.
        tx.wake_after(std::time::Duration::from_secs(1));
    }
}

/// Which conversations show a "Moved to the background" note.
pub(super) fn escalated(ui: &Ui, id: &ConversationId) -> bool {
    ui.chats.escalated.contains(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_names_its_folder_and_links() {
        let answer = "Done. Everything is in `~/Hex/Jobs/suppliers/` and the sheet is at https://docs.google.com/spreadsheets/d/abc123/edit.";
        assert_eq!(folder_in(answer).as_deref(), Some("~/Hex/Jobs/suppliers/"));
        assert_eq!(
            links_in(answer),
            vec!["https://docs.google.com/spreadsheets/d/abc123/edit".to_owned()]
        );
        assert_eq!(
            folder_in("Saved the report to /Users/me/Hex/Jobs/q3/report.md.").as_deref(),
            Some("/Users/me/Hex/Jobs/q3/")
        );
        assert_eq!(folder_in("Nothing here, and no / alone"), None);
        assert!(worth_a_row("Suppliers.xlsx") && !worth_a_row(".DS_Store"));
    }

    #[test]
    fn a_question_about_google_offers_its_sign_in() {
        assert_eq!(
            sign_in_for("I need you to sign in to Gmail first."),
            Some(Some("https://accounts.google.com"))
        );
        assert_eq!(
            sign_in_for("Please log in to the supplier portal"),
            Some(None)
        );
        assert_eq!(sign_in_for("Which 50 suppliers?"), None);
    }

    #[test]
    fn a_job_folder_with_spaces_is_found_whole() {
        let folder = "/Users/felix/Documents/Hex/2026-11-12 Hex selbst testen und Lücken notieren";
        let answer = format!("Alles liegt im Ordner {folder}.");
        let is_dir = |path: &str| path == folder;
        assert_eq!(
            folder_in_with(&answer, &is_dir).as_deref(),
            Some(format!("{folder}/").as_str())
        );
    }

    #[test]
    fn only_result_links_get_buttons() {
        let answer = "Quellen: https://www.obelink.nl/x und https://coolblue.nl/y. Tabelle: https://docs.google.com/spreadsheets/d/abc";
        assert_eq!(
            links_in(answer),
            ["https://docs.google.com/spreadsheets/d/abc"]
        );
    }
}
