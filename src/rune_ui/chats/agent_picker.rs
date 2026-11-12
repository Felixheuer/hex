//! The agent picker, for the onboarding's agent page and Settings: one
//! quiet list (each agent's name, why you'd pick it, a status chip, and
//! one action that does everything its state needs, with inline progress),
//! a suggestion with one button when nothing is installed, and the
//! own-key panel for OpenCode.

use rune::{El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, RadioItem, Spinner, TextInput};

use super::super::controls::{Paint, PushSize};
use super::super::icons::SPARKLES;
use super::super::lists::{Tone, tone_tag};
use super::super::platform::Os;
use super::agents::{self, Status};
use super::{Ui, backend_name};
use crate::app_settings::AssistantBackend;
use crate::desktop::i18n::{tr, tr_fill};

static CHECK_ICON: &rune::Icon = &rune_theme_opencode::icons::CIRCLE_CHECK;

/// Why you'd pick `backend`, in one line.
fn why(backend: AssistantBackend) -> &'static str {
    match backend {
        AssistantBackend::ClaudeCode => tr("Best on the desktop · your Claude plan"),
        AssistantBackend::Codex => tr("Your ChatGPT plan"),
        AssistantBackend::OpenCode => tr("Any model with your own key"),
        AssistantBackend::ClaudeApi => tr("Anthropic key, pay as you go"),
    }
}

fn chip(p: Paint, status: Status) -> Option<El> {
    let (label, tone) = match status {
        Status::Ready => (tr("Ready"), Tone::Success),
        Status::SignIn => (tr("Signed out"), Tone::Warning),
        Status::Install => (tr("Not installed"), Tone::Neutral),
        Status::AddKey => (tr("No key yet"), Tone::Neutral),
        Status::Unknown => return None,
    };
    Some(tone_tag(p, "status", label, tone))
}

/// The one action for `backend` in `status`.
fn action_label(backend: AssistantBackend, status: Status) -> Option<&'static str> {
    Some(match (backend, status) {
        (_, Status::Ready | Status::Unknown) => return None,
        (AssistantBackend::OpenCode, _) => tr("Set up…"),
        (_, Status::Install) => tr("Install"),
        (_, Status::SignIn) => tr("Sign in…"),
        (_, Status::AddKey) => tr("Add key"),
    })
}

/// A spinner and the setup's latest line.
fn progress(ui: &Ui, p: Paint, fallback: &str) -> El {
    let line = ui
        .chats
        .settings
        .agents
        .log
        .last()
        .cloned()
        .unwrap_or_else(|| fallback.to_owned());
    row()
        .key("progress")
        .role("status")
        .label(line.clone())
        .gap(8.0)
        .center_y()
        .child(
            Spinner::new()
                .size(12.0)
                .color(p.accent())
                .build(p.k)
                .a11y_hidden(),
        )
        .child(
            text(line)
                .key("line")
                .font(p.k.ty.small)
                .color(p.muted())
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
}

/// One agent: its radio (name and why), status chip, and one action, with
/// what it's doing under it.
fn agent_row(ui: &Ui, p: Paint, index: usize, backend: AssistantBackend) -> El {
    let state = &ui.chats.settings.agents;
    let selected = ui.settings.assistant.backend == backend;
    let status = agents::status(agents::found(ui, backend));
    let working = state.working == Some(backend);
    let pick = backend;
    let radio = RadioItem::new(backend_name(backend), selected)
        .description(why(backend))
        .on_select_tx(move |ui: &mut Ui, tx: &mut Tx| {
            tx.instant();
            ui.update(|settings| settings.assistant.backend = pick);
        })
        .build(p.k)
        .key("radio")
        .grow(1.0)
        .shrink();
    let action = action_label(backend, status)
        .filter(|_| !working)
        .map(|label| {
            Button::new(label)
                .variant(if selected {
                    ButtonVariant::Contrast
                } else {
                    ButtonVariant::Neutral
                })
                .disabled(state.working.is_some())
                .on_press(move |ui: &mut Ui| agents::act(ui, backend))
                .build_small_push(p.k)
                .key("act")
        });
    let mut line = row()
        .key("line")
        .gap(10.0)
        .center_y()
        .child(radio)
        .children(chip(p, status))
        .children(action);
    if working {
        line = line.child(
            el().key("busy")
                .size(16.0, 16.0)
                .center()
                .child(Spinner::new().size(14.0).color(p.accent()).build(p.k)),
        );
    }
    let mut block = col()
        .key(format!("agent{index}"))
        .role("group")
        .label(backend_name(backend))
        .px(12.0)
        .py(8.0)
        .gap(6.0)
        .radius(p.k.radius.md)
        .child(line);
    if working {
        block = block.child(progress(ui, p, tr("Setting up…")).pl(30.0));
    }
    if let Some((failed, why)) = &state.failed
        && *failed == backend
    {
        block = block.child(
            super::sheet::install_failure(p, why, state.command.as_deref())
                .w(0.0)
                .grow(1.0)
                .pl(30.0),
        );
    }
    match backend {
        AssistantBackend::Codex if status != Status::Ready && !working => {
            block = block.child(codex_key(ui, p));
        }
        AssistantBackend::ClaudeApi if state.api_key_open && status != Status::Ready => {
            block = block.child(api_key(ui, p));
        }
        _ => {}
    }
    block
}

/// Codex: "Use an OpenAI API key instead", then its field.
fn codex_key(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings.agents;
    let Some(field) = &state.codex_key else {
        return link(
            p,
            "codex-use-key",
            tr("Use an OpenAI API key instead"),
            |ui| {
                ui.chats.settings.agents.codex_key = Some(rune::TextEdit::default());
            },
        )
        .pl(30.0);
    };
    let mut block = col().key("codex-key").pl(30.0).gap(6.0).child(
        text(if ui.platform.caps.os == Os::Macos {
            tr("This replaces Codex's ChatGPT sign-in on this Mac for all apps.")
        } else {
            tr("This replaces Codex's ChatGPT sign-in on this computer for all apps.")
        })
        .key("warning")
        .role("status")
        .font(p.k.ty.small_medium)
        .color(p.warning())
        .wrap(),
    );
    block = block.child(
        row()
            .key("field-row")
            .gap(8.0)
            .center_y()
            .child(
                TextInput::new(field)
                    .secure()
                    .placeholder("sk-…")
                    .label(tr("OpenAI API key"))
                    .width(None)
                    .on_edit(|ui: &mut Ui| {
                        ui.chats
                            .settings
                            .agents
                            .codex_key
                            .get_or_insert_with(Default::default)
                    })
                    .on_submit(|ui: &mut Ui, _| agents::use_codex_key(ui))
                    .build(p.k)
                    .key("codex-key-field")
                    .grow(1.0)
                    .shrink(),
            )
            .child(
                Button::new(tr("Use key"))
                    .variant(ButtonVariant::Neutral)
                    .on_press(agents::use_codex_key)
                    .build_small_push(p.k)
                    .key("codex-use"),
            ),
    );
    if let Some(error) = &state.codex_error {
        block = block.child(
            text(error.clone())
                .key("error")
                .font(p.k.ty.small)
                .color(p.danger())
                .wrap(),
        );
    }
    block
}

/// The Claude API key, saved to the Keychain.
fn api_key(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings;
    let mut block = col().key("api-key").pl(30.0).gap(6.0).child(
        row()
            .key("field-row")
            .gap(8.0)
            .center_y()
            .child(
                TextInput::new(&state.key)
                    .secure()
                    .placeholder("sk-ant-…")
                    .label(tr("Claude API key"))
                    .width(None)
                    .on_edit(|ui: &mut Ui| &mut ui.chats.settings.key)
                    .on_submit(|ui: &mut Ui, _| super::settings::save_key(ui))
                    .invalid(state.key_error.is_some())
                    .build(p.k)
                    .key("api-key-field")
                    .grow(1.0)
                    .shrink(),
            )
            .child(
                Button::new(tr("Save"))
                    .variant(ButtonVariant::Contrast)
                    .on_press(super::settings::save_key)
                    .build_small_push(p.k)
                    .key("save-api-key"),
            ),
    );
    if let Some(error) = &state.key_error {
        block = block.child(
            text(error.clone())
                .key("error")
                .font(p.k.ty.small)
                .color(p.danger())
                .wrap(),
        );
    }
    block
}

/// A quiet text link.
fn link(p: Paint, key: &str, label: &str, press: impl Fn(&mut Ui) + 'static) -> El {
    text(label.to_owned())
        .key(key.to_owned())
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .focus_ring(2.0, 0.0, p.k.c.border_focus)
        .font(p.k.ty.small_medium)
        .color(p.accent())
        .py(4.0)
        .radius(p.k.radius.sm)
        .on_press(press)
        .self_start()
}

/// A small choice chip.
pub(super) fn choice(
    p: Paint,
    key: String,
    label: &str,
    on: bool,
    press: impl Fn(&mut Ui) + 'static,
) -> El {
    let k = p.k;
    let chip = row()
        .key(key)
        .role("radio")
        .label(label.to_owned())
        .checked(on)
        .focusable(true)
        .focus_ring(2.0, 0.0, k.c.border_focus)
        .h(26.0)
        .px(10.0)
        .center_y()
        .radius(13.0)
        .on_press(press)
        .child(
            text(label.to_owned())
                .key("label")
                .font(k.ty.small_medium)
                .color(if on { p.text() } else { p.muted() })
                .a11y_hidden(),
        );
    if on {
        chip.fill(k.c.layer_02).border(1.0, k.c.border_strong)
    } else {
        chip.border(p.hair, k.c.border_weak).hover(k.c.hover)
    }
}

/// OpenCode on your own key: provider, key (with where to get one), model,
/// and Verify, which installs OpenCode first when it isn't.
fn byok(ui: &Ui, p: Paint) -> Option<El> {
    let state = &ui.chats.settings.agents;
    let byok = state.byok.as_ref()?;
    let provider = agents::provider(byok);
    let k = p.k;
    let providers = crate::assistant::backends::providers();
    let mut provider_chips = row()
        .key("providers")
        .role("radiogroup")
        .gap(6.0)
        .flex_wrap();
    for (index, each) in providers.iter().enumerate() {
        provider_chips = provider_chips.child(choice(
            p,
            format!("provider-{}", each.id),
            each.name,
            index == byok.provider,
            move |ui: &mut Ui| {
                if let Some(byok) = ui.chats.settings.agents.byok.as_mut() {
                    byok.provider = index;
                    byok.model = 0;
                    byok.result = None;
                }
            },
        ));
    }
    let mut model_chips = row().key("models").role("radiogroup").gap(6.0).flex_wrap();
    for (index, model) in provider.models.iter().enumerate() {
        model_chips = model_chips.child(choice(
            p,
            format!("model{index}"),
            model,
            index == byok.model,
            move |ui: &mut Ui| {
                if let Some(byok) = ui.chats.settings.agents.byok.as_mut() {
                    byok.model = index;
                    byok.result = None;
                }
            },
        ));
    }
    let key_url = provider.key_url;
    let installed =
        agents::found(ui, AssistantBackend::OpenCode).is_some_and(|found| found.path.is_some());
    let verify_label = if installed {
        tr("Verify")
    } else {
        tr("Install OpenCode and verify")
    };
    let verified = matches!(byok.result, Some(Ok(_)));
    let verify = if byok.verifying {
        progress(ui, p, tr("Checking the key…"))
    } else {
        Button::new(verify_label)
            .variant(if verified {
                ButtonVariant::Neutral
            } else {
                ButtonVariant::Contrast
            })
            .disabled(byok.key.text.trim().is_empty())
            .on_press(agents::verify)
            .build_small_push(k)
            .key("verify-key")
    };
    let result = byok.result.as_ref().map(|result| match result {
        Ok(model) => row()
            .key("verified")
            .role("status")
            .gap(6.0)
            .center_y()
            .child(icon(CHECK_ICON, 14.0).color(p.success()).a11y_hidden())
            .child(
                text(tr_fill("Works. OpenCode answers with {}", model))
                    .key("words")
                    .font(k.ty.small_medium)
                    .color(p.success()),
            ),
        Err(why) => text(why.clone())
            .key("verify-error")
            .role("status")
            .font(k.ty.small)
            .color(p.danger())
            .wrap(),
    });
    let label = |key: &str, words: &str| {
        text(words.to_owned())
            .key(key.to_owned())
            .font(k.ty.small_medium)
            .color(p.muted())
    };
    Some(
        col()
            .key("byok")
            .role("group")
            .label(tr("Your own API key"))
            .mx(4.0)
            .mt(4.0)
            .px(12.0)
            .py(12.0)
            .gap(10.0)
            .radius(k.radius.md)
            .fill(k.c.layer_01)
            .border(p.hair, k.c.border_weak)
            .transition(super::super::onboarding_kit::arrives())
            .child(
                col()
                    .key("head")
                    .gap(2.0)
                    .child(
                        text(tr("Your own API key"))
                            .key("title")
                            .font(k.ty.ui_medium)
                            .color(p.text()),
                    )
                    .child(
                        text(match ui.platform.caps.os {
                            Os::Macos => tr("OpenCode runs any provider's model on your key. Hex keeps the key in your Keychain."),
                            Os::Windows => tr("OpenCode runs any provider's model on your key. Hex keeps the key in Windows Credential Manager."),
                            Os::Linux => tr("OpenCode runs any provider's model on your key. Hex keeps the key in your keyring."),
                        })
                            .key("detail")
                            .font(k.ty.small)
                            .color(p.muted())
                            .wrap(),
                    ),
            )
            .child(label("provider-label", tr("Provider")))
            .child(provider_chips)
            .child(
                row()
                    .key("key-row")
                    .gap(8.0)
                    .center_y()
                    .child(
                        TextInput::new(&byok.key)
                            .secure()
                            .placeholder(tr_fill("Your {} key", provider.name))
                            .label(tr("API key"))
                            .width(None)
                            .on_edit(|ui: &mut Ui| {
                                &mut ui
                                    .chats
                                    .settings
                                    .agents
                                    .byok
                                    .get_or_insert_with(Default::default)
                                    .key
                            })
                            .on_submit(|ui: &mut Ui, _| agents::verify(ui))
                            .build(k)
                            .key("byok-key")
                            .grow(1.0)
                            .shrink(),
                    )
                    .child(link(p, "get-key", tr("Get a key"), move |ui| {
                        ui.chats.source.open(key_url, false);
                    })),
            )
            .child(label("model-label", tr("Model")))
            .child(model_chips)
            .child(
                col()
                    .key("verify-row")
                    .gap(8.0)
                    .child(verify.self_start())
                    .children(result),
            ),
    )
}

/// Nothing installed: the suggested agent with one button, then the own-key
/// way and the whole list.
fn suggestion(ui: &Ui, p: Paint) -> El {
    let backend = agents::recommended(ui);
    let k = p.k;
    let working = ui.chats.settings.agents.working == Some(backend);
    let action = if working {
        progress(ui, p, tr("Setting up…"))
    } else {
        Button::new(tr_fill("Set up {}", backend_name(backend)))
            .variant(ButtonVariant::Contrast)
            .on_press(move |ui: &mut Ui| {
                ui.update(|settings| settings.assistant.backend = backend);
                agents::set_up(ui, backend);
            })
            .build_push(k)
            .key("set-up-recommended")
            .default_button()
    };
    let mut card = col()
        .key("suggested")
        .role("group")
        .label(backend_name(backend))
        .mx(4.0)
        .px(16.0)
        .py(16.0)
        .gap(12.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_focus)
        .child(
            row()
                .key("head")
                .gap(10.0)
                .center_y()
                .child(icon(&SPARKLES, 18.0).color(p.accent()).a11y_hidden())
                .child(
                    col()
                        .key("copy")
                        .grow(1.0)
                        .shrink()
                        .gap(2.0)
                        .child(
                            text(tr_fill("Recommended: {}", backend_name(backend)))
                                .key("title")
                                .font(k.ty.ui_medium)
                                .color(p.text()),
                        )
                        .child(
                            text(why(backend))
                                .key("why")
                                .font(k.ty.small)
                                .color(p.muted()),
                        ),
                ),
        )
        .child(action);
    let state = &ui.chats.settings.agents;
    if let Some((failed, why)) = &state.failed
        && *failed == backend
    {
        card = card.child(
            super::sheet::install_failure(p, why, state.command.as_deref())
                .w(0.0)
                .grow(1.0),
        );
    }
    col().key("nothing-installed").gap(4.0).child(card).child(
        row()
            .key("alternatives")
            .px(4.0)
            .gap(4.0)
            .center_y()
            .child(link(
                p,
                "use-own-key",
                tr("Or use your own API key"),
                agents::open_byok,
            ))
            .child(el().key("spacer").grow(1.0))
            .child(link(p, "see-all-agents", tr("See all agents"), |ui| {
                ui.chats.settings.agents.show_all = true;
            })),
    )
}

/// The picker: the suggestion when nothing is installed, else the list,
/// and the own-key panel when open.
pub(super) fn picker(ui: &Ui, p: Paint) -> El {
    let state = &ui.chats.settings.agents;
    let mut picker = col().key("agent-picker").py(4.0).gap(2.0);
    if agents::nothing_installed(ui) && !state.show_all {
        return picker.child(suggestion(ui, p));
    }
    if ui.chats.settings.found.is_none() {
        return picker.child(row().key("looking").px(16.0).py(12.0).child(progress(
            ui,
            p,
            if ui.platform.caps.os == Os::Macos {
                tr("Looking for the agents on this Mac…")
            } else {
                tr("Looking for the agents on this computer…")
            },
        )));
    }
    for (index, backend) in agents::all().into_iter().enumerate() {
        picker = picker.child(agent_row(ui, p, index, backend));
        if backend == AssistantBackend::OpenCode
            && let Some(panel) = byok(ui, p)
        {
            picker = picker.child(panel);
        }
    }
    picker
}
