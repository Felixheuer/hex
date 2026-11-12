//! The onboarding's Privacy page (where your words go, how freely the
//! assistant acts, the apps it never touches) and the optional "Tell Hex
//! about you" page.

use rune::{El, col, icon, row, text};
use rune_kit::TextInput;

use super::super::super::controls::Paint;
use super::super::super::lists::small_button;
use super::super::Ui;
use super::super::setup::{Page, SetupFlow, page_done};
use super::{actions, card, lead, page, separator};
use crate::desktop::i18n::tr;

static SEND_ICON: &rune::Icon = &rune_theme_opencode::icons::ARROW_UP_RIGHT;
static LOCAL_ICON: &rune::Icon = &rune_theme_opencode::icons::LOCK;
static SHIELD_ICON: &rune::Icon = &rune_theme_opencode::icons::SHIELD;
static REMOVE_ICON: &rune::Icon = &rune_theme_opencode::icons::CLOSE_SMALL;

/// One plain-words line: an icon and a sentence.
fn fact(p: Paint, key: &str, glyph: &'static rune::Icon, words: &str) -> El {
    row()
        .key(key.to_owned())
        .gap(10.0)
        .items_start()
        .child(
            icon(glyph, 15.0)
                .key("icon")
                .color(p.k.c.icon_base)
                .mt(2.0)
                .a11y_hidden(),
        )
        .child(
            text(words.to_owned())
                .key("words")
                .font(p.k.ty.body)
                .color(p.text())
                .wrap()
                .grow(1.0)
                .shrink(),
        )
}

/// The blocked apps as removable chips, and a field to add one.
fn blocked(ui: &Ui, p: Paint) -> El {
    let k = p.k;
    let mut chips = row().key("blocked-chips").gap(6.0).flex_wrap();
    for (index, app) in ui.settings.assistant.blocked_apps.iter().enumerate() {
        let remove = app.clone();
        chips = chips.child(
            row()
                .key(format!("blocked{index}"))
                .h(26.0)
                .pl(10.0)
                .pr(4.0)
                .gap(4.0)
                .center_y()
                .radius(13.0)
                .border(p.hair, k.c.border_weak)
                .child(
                    text(app.clone())
                        .key("name")
                        .font(k.ty.small_medium)
                        .color(p.text()),
                )
                .child(super::super::super::list_detail::ghost_icon(
                    p,
                    "remove",
                    REMOVE_ICON,
                    &crate::desktop::i18n::tr_fill("Remove {}", app),
                    move |ui: &mut Ui, _| {
                        let remove = remove.clone();
                        ui.update(|settings| {
                            settings.assistant.blocked_apps.retain(|app| *app != remove)
                        });
                    },
                )),
        );
    }
    let add = |ui: &mut Ui| {
        let app = ui.chats.settings.blocked.text.trim().to_owned();
        if !app.is_empty() && ui.update(|settings| settings.assistant.blocked_apps.push(app)) {
            ui.chats.settings.blocked.clear();
        }
    };
    col()
        .key("blocked")
        .gap(8.0)
        .child(
            text(tr("Blocked apps"))
                .key("title")
                .font(k.ty.ui_medium)
                .color(p.text()),
        )
        .child(chips)
        .child(
            row()
                .key("add")
                .gap(8.0)
                .center_y()
                .child(
                    TextInput::new(&ui.chats.settings.blocked)
                        .placeholder(tr("App name or bundle ID"))
                        .label(tr("Block an app"))
                        .width(None)
                        .on_edit(|ui: &mut Ui| &mut ui.chats.settings.blocked)
                        .on_submit(move |ui: &mut Ui, _| add(ui))
                        .build(k)
                        .key("blocked-field")
                        .grow(1.0)
                        .shrink(),
                )
                .child(small_button(
                    p,
                    "add-blocked-app",
                    tr("Add"),
                    move |ui: &mut Ui, _| add(ui),
                )),
        )
}

/// Where your words go, how freely it acts, and the apps it never touches.
pub(super) fn privacy(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let facts = col()
        .key("facts")
        .gap(10.0)
        .px(16.0)
        .py(14.0)
        .child(fact(
            p,
            "goes",
            SEND_ICON,
            tr("Questions, screenshots, and page content go to the agent you choose (Claude, OpenAI, DeepSeek, …)."),
        ))
        .child(fact(
            p,
            "stays",
            LOCAL_ICON,
            if cfg!(target_os = "macos") {
                tr("Dictation stays on your Mac.")
            } else {
                tr("Dictation stays on your computer.")
            },
        ))
        .child(fact(
            p,
            "never",
            SHIELD_ICON,
            tr("Blocked apps and password fields are never captured."),
        ));
    let body = card(p, "privacy-card")
        .child(facts)
        .child(separator(p, "sep0".into()))
        .child(
            col()
                .key("acts")
                .gap(8.0)
                .px(16.0)
                .py(14.0)
                .child(
                    text(tr("How freely it acts"))
                        .key("title")
                        .font(p.k.ty.ui_medium)
                        .color(p.text()),
                )
                .child(super::super::settings::more::autonomy_choice(ui, p)),
        )
        .child(separator(p, "sep1".into()))
        .child(blocked(ui, p).px(16.0).py(14.0));
    page(
        p,
        "privacy",
        tr("Your privacy, your rules"),
        lead(
            p,
            if cfg!(target_os = "macos") {
                tr(
                    "In plain words: what leaves your Mac, and what the assistant may do on its own.",
                )
            } else {
                tr(
                    "In plain words: what leaves your computer, and what the assistant may do on its own.",
                )
            },
        ),
        Some(body),
        None,
        actions(p, page_done(ui, flow, Page::Privacy), false),
    )
}

/// "Tell Hex about you": company, website, and colleagues, all optional.
pub(super) fn about(ui: &Ui, flow: &SetupFlow, p: Paint) -> El {
    let k = p.k;
    let field = |index: usize, key: &str, label: &str, hint: &str| {
        col()
            .key(key.to_owned())
            .gap(4.0)
            .child(
                text(label.to_owned())
                    .key("label")
                    .font(k.ty.small_medium)
                    .color(p.muted()),
            )
            .child(
                TextInput::new(&flow.about[index])
                    .placeholder(hint.to_owned())
                    .label(label.to_owned())
                    .width(None)
                    .on_edit(move |ui: &mut Ui| {
                        &mut ui.chats.setup.get_or_insert_with(Default::default).about[index]
                    })
                    .build(k)
                    .key("field"),
            )
    };
    let body = card(p, "about-card").child(
        col()
            .key("fields")
            .gap(12.0)
            .px(16.0)
            .py(14.0)
            .child(field(
                0,
                "company",
                tr("Company"),
                tr("Where you work, and what you do there"),
            ))
            .child(field(1, "website", tr("Website"), "acme.com"))
            .child(field(
                2,
                "colleagues",
                tr("Colleagues"),
                tr("Names you'll mention, like Maya (design), Sam (manager)"),
            )),
    );
    page(
        p,
        "about",
        tr("Tell Hex about you"),
        lead(
            p,
            tr(
                "Optional. It remembers these so you don't repeat them. Change them anytime in Settings.",
            ),
        ),
        Some(body),
        None,
        actions(p, page_done(ui, flow, Page::About), true),
    )
}
