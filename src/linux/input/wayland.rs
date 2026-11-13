//! The Wayland backend: Agent-Desk's shortcuts through the desktop's
//! GlobalShortcuts portal, or `agent-desk` commands alone while there is none.

use super::*;

impl Backend {
    pub(super) fn run_wayland(
        self,
        mut dispatcher: Dispatcher,
        binding: LinuxHotkey,
        options: HotkeyOptions,
    ) -> Result<()> {
        let portal = match crate::linux::portal::Portal::connect(&self.stop)? {
            Some(portal) => portal,
            None => {
                tracing::warn!(
                    desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
                    "{}",
                    crate::linux::portal::MISSING_SHORTCUTS_PORTAL
                );
                match self.wait_for_portal(&mut dispatcher, true)? {
                    Some(portal) => portal,
                    None => return Ok(()),
                }
            }
        };
        self.run_portal(portal, dispatcher, binding, options)
    }

    /// Serves `agent-desk` commands, and with `probe` looks for the shortcuts
    /// portal again from time to time: a portal started before the desktop's
    /// session environment exposes none until it is restarted (#875).
    /// Returns the portal once it appears, or `None` when the listener stops.
    pub(super) fn wait_for_portal(
        &self,
        dispatcher: &mut Dispatcher,
        probe: bool,
    ) -> Result<Option<crate::linux::portal::Portal>> {
        self.mark_ready(DesktopShortcuts::Commands);
        let mut probe = probe.then(|| crate::linux::portal::PortalProbe::new(self.stop.clone()));
        while !self.stopping() {
            // No portal: `agent-desk` commands are the only input, so wait on their
            // queue; the probe is due far less often than this.
            dispatcher.wait_for_commands(COMMAND_WAIT);
            if !dispatcher.flush()? {
                return Ok(None);
            }
            if let Some(probe) = probe.as_mut()
                && let Some(portal) = probe.poll()?
            {
                return Ok(Some(portal));
            }
        }
        Ok(None)
    }

    fn run_portal(
        self,
        mut portal: crate::linux::portal::Portal,
        mut dispatcher: Dispatcher,
        binding: LinuxHotkey,
        options: HotkeyOptions,
    ) -> Result<()> {
        let requested = [
            Some((Shortcut::Dictation, &binding)),
            options
                .paste_last
                .as_ref()
                .map(|key| (Shortcut::PasteLast, key)),
            options.capture.as_ref().map(|key| (Shortcut::Capture, key)),
            options
                .raw_dictation
                .as_ref()
                .map(|key| (Shortcut::RawDictation, key)),
            options
                .typed_text
                .as_ref()
                .map(|key| (Shortcut::TypedText, key)),
            options
                .terminal_paste
                .as_ref()
                .map(|key| (Shortcut::TerminalPaste, key)),
            options
                .second_dictation
                .as_ref()
                .map(|key| (Shortcut::SecondDictation, key)),
            options.action.as_ref().map(|key| (Shortcut::Action, key)),
            options
                .stop_all
                .as_ref()
                .map(|key| (Shortcut::StopAll, key)),
        ];
        let desktop_assigns = crate::linux::portal::desktop_assigns_triggers();
        let mut shortcuts = Vec::new();
        let mut ids: [Option<String>; Shortcut::ALL.len()] = Default::default();
        let mut legacy_ids: [Option<String>; Shortcut::ALL.len()] = Default::default();
        for (shortcut, key) in requested.into_iter().flatten() {
            let (base, description) = shortcut.portal_id();
            // Keep keyboard shortcuts usable after moving X11 settings to
            // Wayland. Mouse actions remain available through the CLI.
            if mouse_button(&key.key).is_some() {
                continue;
            }
            let trigger = key.portal_trigger()?;
            let id = portal_shortcut_id(base, &trigger, desktop_assigns);
            ids[shortcut.index()] = Some(id.clone());
            shortcuts.push(crate::linux::portal::Shortcut {
                id,
                description,
                trigger: Some(trigger.clone()),
            });
            if desktop_assigns && let Some(legacy) = hyprland_legacy_id(shortcut, &trigger) {
                legacy_ids[shortcut.index()] = Some(legacy.clone());
                shortcuts.push(crate::linux::portal::Shortcut {
                    id: legacy,
                    description,
                    trigger: Some(trigger),
                });
            }
        }
        // Every shortcut goes in this one bind: a second bind would replace
        // the set on KDE. Escape is never among them (#786), so on Wayland
        // Escape never reaches the assistant; Stop all tasks does.
        // On KDE, ids of shortcuts changed since keep holding their keys,
        // which a new id could then never take; they go first (#898).
        let keep = shortcuts
            .iter()
            .map(|shortcut| shortcut.id.as_str())
            .collect::<Vec<_>>();
        if let Err(error) = portal.forget_legacy_kde_shortcuts(&keep) {
            tracing::warn!(
                error = format!("{error:#}"),
                "could not remove Agent-Desk's old shortcuts from KDE"
            );
        }
        // GNOME asks for approval whenever Agent-Desk binds shortcuts it hasn't
        // approved, as after a shortcut change; dismissing that dialog fails
        // the bind (#723), and Enable binds again, so the dialog returns.
        let granted = match portal.bind(&shortcuts, &self.stop) {
            Ok(granted) => granted,
            Err(error) if self.stopping() => return Err(error),
            // The backend failed the bind rather than the person refusing it
            // (niri with the GNOME backend and no GNOME Shell, #1014): treat
            // the desktop as having no usable shortcuts portal. Binding again
            // would fail the same way, so this run doesn't look for it again;
            // a listener restart (Enable, a settings change) tries once more.
            Err(error) if crate::linux::portal::is_backend_failure(&error) => {
                tracing::warn!(
                    error = format!("{error:#}"),
                    desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
                    "the shortcuts portal's backend could not bind Agent-Desk's shortcuts; dictation follows the agent-desk commands, which reach the service listener (`agent-desk start`) but not a foreground `agent-desk dictate`"
                );
                drop(portal);
                self.wait_for_portal(&mut dispatcher, false)?;
                return Ok(());
            }
            Err(error) => {
                return Err(error.wrap_err(
                    "the desktop did not approve Agent-Desk's shortcuts; choose Enable to ask again",
                ));
            }
        };
        let project = |granted: &std::collections::HashMap<String, Option<String>>| {
            project_grants(granted, &requested, &ids, desktop_assigns)
        };
        let modifiers = |granted: &std::collections::HashMap<String, Option<String>>| {
            Shortcut::ALL.map(|shortcut| {
                requested.iter().flatten().any(|(requested, key)| {
                    *requested == shortcut
                        && trigger_has_modifiers(granted_trigger(granted, &ids, shortcut), key)
                })
            })
        };
        let mut with_modifiers = modifiers(&granted);
        let granted = project(&granted);
        for shortcut in &granted {
            match &shortcut.trigger {
                None if shortcut.unconfirmed => {
                    tracing::info!(
                        name = %shortcut.name,
                        "the desktop assigns this Agent-Desk shortcut's key itself; Agent-Desk cannot confirm it"
                    );
                }
                None => {
                    tracing::warn!(
                        name = %shortcut.name,
                        "the desktop did not grant an Agent-Desk shortcut"
                    );
                }
                Some(_) => {}
            }
        }
        self.status
            .configurable
            .store(portal.configurable(), Ordering::Release);
        self.status.configure.store(false, Ordering::Release);
        self.mark_ready(DesktopShortcuts::Portal(granted));
        let shortcut_for = |id: &str| {
            Shortcut::ALL.into_iter().find(|shortcut| {
                ids[shortcut.index()].as_deref() == Some(id)
                    || legacy_ids[shortcut.index()].as_deref() == Some(id)
            })
        };
        while !self.stopping() {
            // Wakes on the desktop's signals; the timeout only lets queued
            // `agent-desk` commands through while nothing is pressed.
            for event in portal.wait_events(PORTAL_WAIT) {
                let input = match event {
                    crate::linux::portal::Event::Pressed(id) => match shortcut_for(&id) {
                        Some(shortcut) => {
                            if with_modifiers[shortcut.index()] {
                                dispatcher.keys.pressed(shortcut);
                            }
                            Input::Pressed(shortcut)
                        }
                        None => continue,
                    },
                    crate::linux::portal::Event::Released(id) => match shortcut_for(&id) {
                        Some(shortcut) => {
                            if with_modifiers[shortcut.index()] {
                                dispatcher.keys.released(shortcut);
                            }
                            Input::Released(shortcut)
                        }
                        None => continue,
                    },
                    crate::linux::portal::Event::Changed(changed) => {
                        with_modifiers = modifiers(&changed);
                        self.status
                            .set_shortcuts(DesktopShortcuts::Portal(project(&changed)));
                        continue;
                    }
                    crate::linux::portal::Event::Closed => {
                        dispatcher.keys.released_all();
                        dispatcher.input(Input::Lost);
                        let _ = dispatcher.flush();
                        return Err(eyre!(
                            "the desktop closed Agent-Desk's shortcuts portal session"
                        ));
                    }
                };
                dispatcher.input(input);
            }
            dispatcher.expire();
            dispatcher.commands();
            if self.status.configure.swap(false, Ordering::AcqRel)
                && let Err(error) = portal.configure()
            {
                // The portal advertises ConfigureShortcuts even when the
                // desktop's backend lacks it (GNOME, #723), so the call
                // fails and opens nothing. Settings then drops its Open
                // button rather than keep one that does nothing.
                self.status.configurable.store(false, Ordering::Release);
                tracing::warn!(
                    error = format!("{error:#}"),
                    "could not open the desktop's shortcut settings"
                );
            }
            if !dispatcher.flush()? {
                break;
            }
        }
        if self.status.forget.load(Ordering::Acquire)
            && let Err(error) = portal.forget()
        {
            tracing::warn!(
                error = format!("{error:#}"),
                "could not clear Agent-Desk's shortcuts from the desktop"
            );
        }
        Ok(())
    }
}
