//! Scheduled requests in Chats: each with when it runs next, a switch, Run
//! now, and Delete. They're made by asking ("every morning at 8, summarize
//! my inbox"), or here with New schedule: what to do, a time, and how
//! often.

use rune::{El, TextEdit, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Switch, TextInput};

use super::super::controls::{Paint, PushSize};
use super::super::list_detail::{self as ld};
use super::bridge::schedules::{Repeat, Schedule};
use super::{ChatsPane, Ui};
use crate::desktop::i18n::tr;

static RUN_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_RIGHT;

const REPEATS: [Repeat; 4] = [Repeat::Once, Repeat::Daily, Repeat::Weekdays, Repeat::Weekly];

/// The New schedule sheet, while it shows.
pub(in crate::rune_ui) struct NewSchedule {
    pub(in crate::rune_ui) text: TextEdit,
    pub(super) time: TextEdit,
    pub(super) repeat: Repeat,
    pub(super) error: Option<String>,
}

impl Default for NewSchedule {
    fn default() -> Self {
        Self {
            text: TextEdit::default(),
            time: TextEdit::new("08:00"),
            repeat: Repeat::Daily,
            error: None,
        }
    }
}

fn repeat_label(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Once => tr("Once"),
        Repeat::Daily => tr("Every day"),
        Repeat::Weekdays => tr("Weekdays"),
        Repeat::Weekly => tr("Every week"),
        Repeat::Custom => tr("Custom"),
    }
}

fn clock(minute_of_day: u32) -> String {
    format!("{}:{:02}", minute_of_day / 60, minute_of_day % 60)
}

/// "8:00" or "08:30" as minutes after midnight.
pub(super) fn parse_time(text: &str) -> Option<u32> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let hours: u32 = hours.trim().parse().ok()?;
    let minutes: u32 = minutes.trim().parse().ok()?;
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

impl ChatsPane {
    /// Runs schedule `id` now, as a new Background conversation.
    pub(super) fn run_schedule(&mut self, id: &str) {
        let Some(text) = self.source.run_schedule(id) else {
            return;
        };
        let mode = std::mem::replace(&mut self.lane, super::SendMode::Background);
        let typed = std::mem::replace(&mut self.composer, TextEdit::new(text));
        self.selected = None;
        self.send();
        self.lane = mode;
        self.composer = typed;
    }

    /// Saves the New schedule sheet.
    pub(super) fn save_schedule(&mut self) {
        let Some(sheet) = self.new_schedule.as_mut() else {
            return;
        };
        let Some(minute) = parse_time(&sheet.time.text) else {
            sheet.error = Some(tr("Use a time like 8:00 or 17:30").into());
            return;
        };
        let text = sheet.text.text.trim().to_owned();
        let repeat = sheet.repeat;
        match self.source.add_schedule(&text, minute, repeat) {
            Ok(()) => {
                self.new_schedule = None;
                self.schedules = self.source.schedules();
            }
            Err(error) => {
                if let Some(sheet) = self.new_schedule.as_mut() {
                    sheet.error = Some(error);
                }
            }
        }
    }
}

fn schedule_row(p: Paint, index: usize, schedule: &Schedule) -> El {
    let toggle = schedule.id.clone();
    let run = schedule.id.clone();
    let remove = schedule.id.clone();
    let on = schedule.enabled;
    let when = format!(
        "{} · {}",
        repeat_label(schedule.repeat),
        clock(schedule.minute_of_day)
    );
    row()
        .key(format!("schedule{index}"))
        .role("listitem")
        .label(schedule.text.clone())
        .description(when.clone())
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .radius(p.k.radius.md)
        .hover(p.k.c.hover)
        .hover_group()
        .child(
            col()
                .key("copy")
                .grow(1.0)
                .w(0.0)
                .gap(2.0)
                .child(
                    text(schedule.text.clone())
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(if on { p.text() } else { p.muted() })
                        .truncate()
                        .a11y_hidden(),
                )
                .child(
                    text(when)
                        .key("when")
                        .font(p.k.ty.small)
                        .color(p.muted())
                        .truncate()
                        .a11y_hidden(),
                ),
        )
        .child(
            row()
                .key("hover")
                .gap(2.0)
                .group_opacity(0.0, 1.0)
                .child(ld::row_action(p, "run-now", RUN_ICON, tr("Run now"), move |ui: &mut Ui, _| {
                    ui.chats.run_schedule(&run);
                }))
                .child(ld::row_action(
                    p,
                    "delete-schedule",
                    ld::TRASH_ICON,
                    tr("Delete schedule"),
                    move |ui: &mut Ui, _| {
                        ui.chats.source.remove_schedule(&remove);
                        ui.chats.schedules = ui.chats.source.schedules();
                    },
                )),
        )
        .child(
            Switch::new(on)
                .label(tr("On"))
                .hide_label()
                .on_toggle(move |ui: &mut Ui| {
                    ui.chats.source.set_schedule_enabled(&toggle, !on);
                    ui.chats.schedules = ui.chats.source.schedules();
                })
                .build(p.k)
                .key("schedule-switch"),
        )
}

/// The list's Scheduled group: its schedules and New schedule.
pub(super) fn group(ui: &Ui, p: Paint) -> El {
    let schedules = &ui.chats.schedules;
    let mut group = col()
        .key("scheduled")
        .role("list")
        .label(tr("Scheduled"))
        .pb(6.0)
        .child(
            row()
                .key("scheduled-head")
                .center_y()
                .child(ld::group(p, "scheduled-title", tr("Scheduled"), true).grow(1.0))
                .child(ld::ghost_icon(
                    p,
                    "new-schedule",
                    &rune_theme_opencode::icons::PLUS_SMALL,
                    tr("New schedule"),
                    |ui: &mut Ui, tx: &mut Tx| {
                        ui.chats.new_schedule = Some(NewSchedule::default());
                        tx.focus("schedule-text", false);
                    },
                )),
        );
    if schedules.is_empty() {
        group = group.child(
            text(tr("Ask “every morning at 8, …” to schedule something"))
                .key("scheduled-hint")
                .font(p.k.ty.small)
                .color(p.faint())
                .wrap()
                .px(12.0)
                .py(4.0),
        );
    }
    for (index, schedule) in schedules.iter().enumerate() {
        group = group.child(schedule_row(p, index, schedule));
    }
    group
}

/// The New schedule sheet over the pane, while it shows.
pub(super) fn sheet(ui: &Ui, p: Paint) -> Option<El> {
    let sheet = ui.chats.new_schedule.as_ref()?;
    let k = p.k;
    let mut repeats = row().key("repeats").role("radiogroup").gap(6.0).flex_wrap();
    for (index, repeat) in REPEATS.into_iter().enumerate() {
        repeats = repeats.child(super::agent_picker::choice(
            p,
            format!("repeat{index}"),
            repeat_label(repeat),
            sheet.repeat == repeat,
            move |ui: &mut Ui| {
                if let Some(sheet) = ui.chats.new_schedule.as_mut() {
                    sheet.repeat = repeat;
                }
            },
        ));
    }
    let card = col()
        .key("schedule-card")
        .role("dialog")
        .label(tr("New schedule"))
        .w(420.0)
        .px(20.0)
        .py(20.0)
        .gap(12.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(p.hair, k.c.border_weak)
        .on_press(|_: &mut Ui| {})
        .child(
            text(tr("New schedule"))
                .key("title")
                .role("heading")
                .font(k.ty.ui_medium)
                .color(p.text()),
        )
        .child(
            TextInput::new(&sheet.text)
                .placeholder(tr("What should it do? For example: summarize my inbox"))
                .label(tr("What to do"))
                .width(None)
                .on_edit(|ui: &mut Ui| {
                    &mut ui.chats.new_schedule.get_or_insert_with(Default::default).text
                })
                .on_submit(|ui: &mut Ui, _| ui.chats.save_schedule())
                .build(k)
                .key("schedule-text"),
        )
        .child(
            row()
                .key("when")
                .gap(10.0)
                .center_y()
                .child(
                    TextInput::new(&sheet.time)
                        .placeholder("8:00")
                        .label(tr("Time"))
                        .width(Some(80.0))
                        .on_edit(|ui: &mut Ui| {
                            &mut ui.chats.new_schedule.get_or_insert_with(Default::default).time
                        })
                        .build(k)
                        .key("schedule-time"),
                )
                .child(repeats.grow(1.0)),
        )
        .children(sheet.error.as_ref().map(|error| {
            text(error.clone())
                .key("error")
                .role("status")
                .font(k.ty.small)
                .color(p.danger())
        }))
        .child(
            row()
                .key("actions")
                .gap(8.0)
                .justify_end()
                .child(
                    Button::new(tr("Cancel"))
                        .variant(ButtonVariant::Ghost)
                        .on_press(|ui: &mut Ui| ui.chats.new_schedule = None)
                        .build_small_push(k)
                        .key("cancel-schedule"),
                )
                .child(
                    Button::new(tr("Save"))
                        .variant(ButtonVariant::Contrast)
                        .disabled(sheet.text.text.trim().is_empty())
                        .on_press(|ui: &mut Ui| ui.chats.save_schedule())
                        .build_small_push(k)
                        .key("save-schedule"),
                ),
        );
    Some(
        el().key("schedule-sheet")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(k.c.bg_deep.alpha(0.6))
            .on_press(|ui: &mut Ui| ui.chats.new_schedule = None)
            .child(card)
            .modal(true, |ui: &mut Ui| ui.chats.new_schedule = None),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn times_read_as_minutes_after_midnight() {
        assert_eq!(super::parse_time("8:00"), Some(480));
        assert_eq!(super::parse_time(" 17:30 "), Some(1050));
        assert_eq!(super::parse_time("24:00"), None);
        assert_eq!(super::parse_time("noon"), None);
    }
}
