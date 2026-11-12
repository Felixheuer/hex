//! What Hex reads through AT-SPI, the accessibility bus, on Linux: the
//! focused element's selected text (never the clipboard), where password
//! fields are, whether the bus is on, and which window an agent-desktop
//! window id names. Every read is bounded by a deadline.

use std::time::Duration;

use zbus::{Connection, OwnedObjectPath, OwnedVariant};

use crate::linux::atspi::{
    self, ACCESSIBLE, MATCH_ALL, MATCH_ANY, MatchRule, Reference, SORT_CANONICAL, STATE_SHOWING,
    WindowHint,
};

const STATE_FOCUSED: u32 = 12;
const ROLE_PASSWORD_TEXT: u32 = 40;
const TEXT: &str = "org.a11y.atspi.Text";
const COMPONENT: &str = "org.a11y.atspi.Component";
/// Extents relative to the screen.
const COORD_SCREEN: u32 = 0;
/// How much of a selection is read, in characters.
const MAX_SELECTION: i32 = 40_000;

/// The text selected in the focused element of the window `hint` names;
/// never a password field's.
pub fn selected_text(hint: &WindowHint, deadline: Duration) -> Option<String> {
    atspi::within(deadline, async {
        let bus = atspi::bus().await?;
        let (window, _) = atspi::window_matching(&bus, hint).await?;
        let focused = first_match(&bus, &window, focused_rule()).await?;
        let role: u32 = atspi::call(
            &bus,
            &focused.0,
            focused.1.as_str(),
            ACCESSIBLE,
            "GetRole",
            &(),
        )
        .await
        .unwrap_or_default();
        if role == ROLE_PASSWORD_TEXT {
            return None;
        }
        let path = focused.1.as_str();
        let count: i32 = atspi::call(&bus, &focused.0, path, TEXT, "GetNSelections", &()).await?;
        let mut text = String::new();
        for index in 0..count.min(8) {
            let (start, end): (i32, i32) =
                atspi::call(&bus, &focused.0, path, TEXT, "GetSelection", &(index,)).await?;
            let end = end.min(start.saturating_add(MAX_SELECTION));
            if end > start {
                let part: String =
                    atspi::call(&bus, &focused.0, path, TEXT, "GetText", &(start, end)).await?;
                text.push_str(&part);
            }
        }
        Some(text).filter(|text| !text.trim().is_empty())
    })
    .flatten()
}

/// The password fields showing in the window `hint` names, in screen
/// coordinates (X11's; toolkits that don't know their position report
/// none).
pub fn password_fields(hint: &WindowHint, deadline: Duration) -> Vec<[f64; 4]> {
    atspi::within(deadline, async {
        let bus = atspi::bus().await?;
        let (window, _) = atspi::window_matching(&bus, hint).await?;
        let fields: Vec<Reference> = atspi::call(
            &bus,
            &window.0,
            window.1.as_str(),
            "org.a11y.atspi.Collection",
            "GetMatches",
            &(password_rule(), SORT_CANONICAL, 32_i32, true),
        )
        .await?;
        let mut rects = Vec::new();
        for (name, path) in fields {
            let extents: Option<(i32, i32, i32, i32)> = atspi::call(
                &bus,
                &name,
                path.as_str(),
                COMPONENT,
                "GetExtents",
                &(COORD_SCREEN,),
            )
            .await;
            if let Some((x, y, width, height)) =
                extents.filter(|extents| extents.2 > 0 && extents.3 > 0)
            {
                rects.push([
                    f64::from(x),
                    f64::from(y),
                    f64::from(width),
                    f64::from(height),
                ]);
            }
        }
        Some(rects)
    })
    .flatten()
    .unwrap_or_default()
}

/// The process and title of the window an agent-desktop id names, when the
/// id is the window's accessible address (`:1.42/org/a11y/atspi/…`).
pub fn window_of(id: &str, deadline: Duration) -> Option<(Option<u32>, Option<String>)> {
    let (name, path) = super::super::atspi_address(id)?;
    atspi::within(deadline, async move {
        let bus = atspi::bus().await?;
        let path = OwnedObjectPath::try_from(path).ok()?;
        let pid = atspi::process_id(&bus, &name).await;
        let title = atspi::accessible_name(&bus, &name, &path).await;
        (pid.is_some() || title.is_some()).then_some((pid, title))
    })
    .flatten()
}

/// Whether the accessibility bus is on (`org.a11y.Status.IsEnabled`).
pub fn enabled(deadline: Duration) -> bool {
    atspi::within(deadline, async {
        let session = Connection::session().await.ok()?;
        let value: OwnedVariant = atspi::call(
            &session,
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.freedesktop.DBus.Properties",
            "Get",
            &("org.a11y.Status", "IsEnabled"),
        )
        .await?;
        bool::try_from(&value).ok()
    })
    .flatten()
    .unwrap_or(false)
}

/// Turns the accessibility bus on, so apps (Chromium, Electron) expose
/// what they show.
pub fn enable(deadline: Duration) -> bool {
    atspi::within(deadline, async {
        let session = Connection::session().await.ok()?;
        session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &("org.a11y.Status", "IsEnabled", zbus::Variant::from(true)),
            )
            .await
            .ok()
            .map(|_| true)
    })
    .flatten()
    .unwrap_or(false)
}

async fn first_match(bus: &Connection, window: &Reference, rule: MatchRule) -> Option<Reference> {
    let found: Vec<Reference> = atspi::call(
        bus,
        &window.0,
        window.1.as_str(),
        "org.a11y.atspi.Collection",
        "GetMatches",
        &(rule, SORT_CANONICAL, 1_i32, true),
    )
    .await?;
    found.into_iter().next()
}

fn focused_rule() -> MatchRule {
    (
        atspi::bitset::<2>(STATE_FOCUSED),
        MATCH_ALL,
        Default::default(),
        MATCH_ALL,
        Vec::new(),
        MATCH_ALL,
        Vec::new(),
        MATCH_ALL,
        false,
    )
}

fn password_rule() -> MatchRule {
    (
        atspi::bitset::<2>(STATE_SHOWING),
        MATCH_ALL,
        Default::default(),
        MATCH_ALL,
        atspi::bitset::<4>(ROLE_PASSWORD_TEXT),
        MATCH_ANY,
        Vec::new(),
        MATCH_ALL,
        false,
    )
}
