//! The General pane: the app itself (appearance, the interface language,
//! launch at login, the Dock and the menu bar, setup, updates, sounds).

use rune::El;

use super::super::controls::{
    ButtonKind, CardRow, Paint, button, icon_row, icon_toggle_row, value_button,
};
use super::super::icons::*;
use super::super::settings as s;
use super::super::{Request, Ui};
use super::{launch_at_login_rows, pick_row, search};
use crate::app_settings::UpdateChannel;
use crate::desktop::host::DesktopAction;
use crate::desktop::i18n::tr;

/// The General pane: permissions and errors, then the app's own settings.
pub(in crate::rune_ui) fn app_pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    use super::super::controls::{page, section};
    use super::super::platform_settings as os;
    let content = page("content")
        .children(os::error_section(ui, p))
        .child(section(
            p,
            "application",
            tr("App"),
            application(ui, p, layer),
        ))
        .child(section(p, "sounds", tr("Sounds"), sound_rows(ui, p, layer)));
    super::super::scroll_body(p.k, "scroll", content)
}

/// Appearance, the interface language, launch at login, the Dock and the
/// menu bar, Enable on launch (Linux and Windows), and the update channel.
fn application(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<CardRow> {
    use crate::app_settings::AppearanceTheme;
    let settings = &ui.settings;
    let mut rows = search::Rows::new("");
    rows.push(
        tr("Appearance"),
        pick_row(
            ui,
            p,
            "appearance",
            &SPARKLES,
            tr("Appearance"),
            AppearanceTheme::ALL
                .iter()
                .map(|theme| tr(theme.name()))
                .collect(),
            AppearanceTheme::ALL
                .iter()
                .position(|theme| *theme == settings.preferences.appearance)
                .unwrap_or(0),
            |ui, index| {
                let theme = AppearanceTheme::ALL[index];
                ui.update(|settings| settings.preferences.appearance = theme);
            },
            layer,
        ),
    );
    let languages = crate::desktop::i18n::LANGUAGE_CHOICES;
    rows.push(
        tr("Interface language"),
        pick_row(
            ui,
            p,
            "interface-language",
            &GLOBE,
            tr("Interface language"),
            languages.iter().map(|(_, name)| tr(name)).collect(),
            languages
                .iter()
                .position(|(code, _)| *code == settings.ui_language.as_deref())
                .unwrap_or(0),
            |ui, index| {
                let code = crate::desktop::i18n::LANGUAGE_CHOICES[index]
                    .0
                    .map(str::to_string);
                if ui.update(|settings| settings.ui_language = code.clone()) {
                    crate::desktop::i18n::apply(code.as_deref());
                }
            },
            layer,
        ),
    );
    rows.extend(tr("Launch at login"), launch_at_login_rows(ui, p));
    if ui.platform.caps.listener_control {
        rows.push(
            tr("Enable on launch"),
            icon_toggle_row(
                p,
                "enable-on-launch",
                &PLAY,
                tr("Enable on launch"),
                None,
                settings.enable_on_launch,
                |ui: &mut Ui| {
                    let enabled = !ui.settings.enable_on_launch;
                    ui.update(|settings| settings.enable_on_launch = enabled);
                },
            ),
        );
    }
    if ui.platform.caps.dock_icon {
        rows.push(
            tr("Show Dock icon"),
            icon_toggle_row(
                p,
                "dock-icon",
                &DOCK,
                tr("Show Dock icon"),
                None,
                settings.show_dock_icon,
                |ui: &mut Ui| {
                    let show = !ui.settings.show_dock_icon;
                    ui.update(|settings| settings.show_dock_icon = show);
                },
            ),
        );
    }
    if ui.platform.caps.os == super::super::platform::Os::Macos {
        rows.push(
            tr("Hide the menu bar icon"),
            icon_toggle_row(
                p,
                "hide-menu-bar-icon",
                &PANEL,
                tr("Hide the menu bar icon"),
                tr("Shortcuts keep working; open Agent-Desk from Spotlight"),
                settings.preferences.hide_menu_bar_icon,
                |ui: &mut Ui| {
                    ui.update(|settings| settings.preferences.hide_menu_bar_icon ^= true);
                },
            ),
        );
        rows.push(
            tr("⌘Q closes the window"),
            icon_toggle_row(
                p,
                "quit-closes-window",
                &POWER,
                tr("⌘Q closes the window"),
                tr("The assistant and dictation keep running; quit from the menu bar"),
                settings.preferences.quit_closes_window,
                |ui: &mut Ui| {
                    ui.update(|settings| settings.preferences.quit_closes_window ^= true);
                },
            ),
        );
    }
    // The first run again: permissions, the assistant, and dictation.
    rows.push(
        tr("Run setup again"),
        icon_row(
            p,
            "setup",
            &REFRESH,
            tr("Setup"),
            Some(tr(
                "Permissions, the assistant, and dictation, checked again",
            )),
            button(
                p,
                "restart-onboarding",
                tr("Run setup again"),
                ButtonKind::Secondary,
                |ui: &mut Ui, tx| {
                    if let Err(error) = super::super::onboarding_kit::live::restart(ui, tx) {
                        ui.error = Some(error);
                    }
                },
            ),
        ),
    );
    if s::updates_itself(ui) {
        let shown = settings
            .updates
            .channel
            .unwrap_or_else(crate::updater::build_channel);
        rows.push(
            tr("Update channel"),
            pick_row(
                ui,
                p,
                "update-channel",
                &REFRESH,
                tr("Update channel"),
                UpdateChannel::ALL
                    .iter()
                    .map(|channel| tr(channel.label()))
                    .collect(),
                UpdateChannel::ALL
                    .iter()
                    .position(|item| *item == shown)
                    .unwrap_or(0),
                |ui, index| {
                    // Choosing the channel already in effect saves nothing, so an
                    // unset channel keeps following the build.
                    let channel = UpdateChannel::ALL[index];
                    let shown = ui
                        .settings
                        .updates
                        .channel
                        .unwrap_or_else(crate::updater::build_channel);
                    if channel != shown
                        && ui.update(|settings| settings.updates.channel = Some(channel))
                    {
                        ui.send(Request::Desktop(DesktopAction::CheckForUpdates));
                    }
                },
                layer,
            ),
        );
    }
    rows.into_rows()
}

/// The sound volume and the sound set.
fn sound_rows(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> Vec<CardRow> {
    let settings = &ui.settings;
    let mut rows = search::Rows::new("");
    rows.push(
        tr("Sound volume"),
        pick_row(
            ui,
            p,
            "sound-volume",
            &VOLUME,
            tr("Sound volume"),
            s::SOUND_VOLUME_STEPS
                .iter()
                .map(|(label, _)| tr(label))
                .collect(),
            s::sound_volume_index(settings),
            |ui, index| {
                let volume = s::SOUND_VOLUME_STEPS[index].1;
                let saved = ui.update(|settings| {
                    settings.sound_effects = volume > 0.0;
                    if volume > 0.0 {
                        settings.sound_effect_volume = volume;
                    }
                });
                if saved && volume > 0.0 && ui.live() {
                    crate::feedback::play(crate::feedback::Tone::DictationStart);
                }
            },
            layer,
        ),
    );
    rows.push(
        tr("Sounds"),
        icon_row(
            p,
            "sound-set",
            &MUSIC,
            tr("Sounds"),
            None,
            value_button(
                p,
                "sound-set-control",
                tr("Sounds"),
                tr(settings.sound_theme.name()),
                &rune_theme_opencode::icons::CHEVRON_RIGHT,
                super::super::sound_sets::open,
            ),
        ),
    );
    rows.into_rows()
}
