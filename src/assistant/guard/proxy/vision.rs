//! The vision path, for any app (`assistant::input`): `hex_window_shot`
//! gives a window's picture wherever it waits (off the screen, behind the
//! person's windows), and `hex_window_input` acts at a spot of that picture
//! by input sent to the app's process: the person's pointer doesn't move,
//! their app keeps the front and the keyboard. Checked like every action:
//! the window must be the task's, a paused task waits, and a click on a
//! consequential control (its label read from the picture: "Senden", "Buy")
//! needs the person's yes unless the autonomy allows it.
//!
//! The same input is the fallback of Accessibility actions: a ref action
//! that fails because the app can't answer (`-25204`, `-25205`, a timeout)
//! is tried once more, then done at the element's spot by pid input, before
//! any window is brought forward. A window that is minimized is first
//! brought back to where it waits (moved off the screen while still
//! minimized, then restored, so it never comes up over the person's work):
//! a minimized window takes no posted input and gives no picture.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use serde_json::{Map, Value, json};

use super::super::child::{Desktop, Window};
use super::super::policy::{Kind, Target, WINDOW_INPUT_TOOL, WINDOW_SHOT_TOOL};
use super::apps::success;
use super::{Guard, Look, Resolved, failure, refusal, text};
use crate::assistant::input::decide::{self, Fallback};
use crate::assistant::input::geometry::{Frame, Units};
use crate::assistant::input::post::{self, Button, Gesture};
use crate::assistant::input::{pointer, shot};

/// The long side of the picture the agent gets (its pixels are the
/// coordinates `hex_window_input` takes); the full-resolution PNG is saved
/// beside it.
const IMAGE_EDGE: u32 = 1568;

/// What the vision path remembers in one task.
#[derive(Default)]
pub(super) struct Vision {
    /// Element frames (global points) by ref, from results.
    frames: HashMap<String, (f64, f64, f64, f64)>,
    /// The last picture given per window number: its size as the agent saw
    /// it, and the full pixels (to tell what changed).
    shots: HashMap<u32, Seen>,
    /// Tests: the text read at any spot (they never read the screen).
    #[cfg(test)]
    pub(in crate::assistant::guard) label: String,
    /// Tests: what was sent to apps (window number, gesture).
    #[cfg(test)]
    pub(in crate::assistant::guard) sent: Vec<(u32, Gesture)>,
}

struct Seen {
    image: (u32, u32),
    #[allow(dead_code)] // Told to the agent with the image.
    full: (u32, u32),
    rgba: Vec<u8>,
}

/// The ref Agent-Desk files a spot's label under, to ask `consent` about it.
const SPOT_REF: &str = "@hexspot:e1";

#[cfg(test)]
impl<D: Desktop> Guard<D> {
    /// Tests: the text the picture "shows" at any spot.
    pub(in crate::assistant::guard) fn set_spot_label(&mut self, label: &str) {
        label.clone_into(&mut self.learned.vision.lock().label);
    }

    /// Tests: what was sent to apps by pid input.
    pub(in crate::assistant::guard) fn sent(&self) -> Vec<(u32, Gesture)> {
        self.learned.vision.lock().sent.clone()
    }
}

impl<D: Desktop> Guard<D> {
    /// Snapshots name where each element is, so a failed Accessibility
    /// action can fall back to a click at its spot.
    pub(super) fn want_bounds(tool: &str, args: &mut Map<String, Value>) {
        if matches!(tool, "desktop_snapshot" | "desktop_find")
            && !args.contains_key("include_bounds")
        {
            args.insert("include_bounds".into(), Value::Bool(true));
        }
    }

    /// Learns element frames from a result's data.
    pub(super) fn learn_frames(&mut self, result: &Value) {
        let Some(data) = super::super::child::data(result) else {
            return;
        };
        let mut found = Vec::new();
        frames_in(&data, &mut found);
        for (reference, frame) in found {
            self.learned.vision.lock().frames.insert(reference, frame);
        }
    }

    /// The window the call names, checked as `kind` (the task's own).
    fn vision_window(
        &mut self,
        tool: &str,
        kind: Kind,
        args: &mut Map<String, Value>,
    ) -> Result<(Resolved, Window), Value> {
        let target = match (text(args, "window_id"), text(args, "app")) {
            (Some(id), _) => Target::Window(id),
            (None, Some(app)) => Target::App(app),
            (None, None) => {
                return Err(refusal(
                    tool,
                    "INVALID_ARGS",
                    "Name the window with window_id (w-1234).",
                ));
            }
        };
        let found = self.resolve(tool, kind, target, args)?;
        let held = self.held();
        for window in &found {
            self.check(tool, kind, window, &held)?;
        }
        let resolved = found
            .into_iter()
            .next()
            .ok_or_else(|| refusal(tool, "WINDOW_NOT_FOUND", "That app has no window to use."))?;
        let id = resolved
            .id
            .clone()
            .ok_or_else(|| refusal(tool, "WINDOW_NOT_FOUND", "That app has no window to use."))?;
        let window = self.window_info(&id).ok_or_else(|| {
            refusal(
                tool,
                "WINDOW_NOT_FOUND",
                &format!("There's no window {id} now; desktop_list_windows shows the open ones."),
            )
        })?;
        Ok((resolved, window))
    }

    /// What the screen says of window `id` (on screen, or off it: minimized,
    /// parked).
    pub(super) fn window_info(&mut self, id: &str) -> Option<Window> {
        let listed = self
            .list(Look::Check)
            .ok()
            .and_then(|windows| windows.into_iter().find(|window| window.id == id))
            .filter(|window| window.pid.is_some() && window.bounds.is_some());
        listed.or_else(|| {
            self.screen
                .off_screen_windows()
                .into_iter()
                .find(|window| window.id == id)
        })
    }

    /// The window's frame (points) and its display's scale.
    fn frame_of(&mut self, window: &Window) -> Option<Frame> {
        let (x, y, width, height) = window.bounds?;
        let displays = self.screen.displays();
        let scale = crate::assistant::computer::display_of(&displays, (x, y, width, height))
            .or_else(|| displays.iter().find(|display| display.is_primary))
            .map_or(2.0, |display| display.scale);
        Some(Frame {
            x,
            y,
            width,
            height,
            scale,
        })
    }

    /// A minimized window of the task comes back to where it waits: moved
    /// off the screen while still minimized, then restored there, so it
    /// never shows over the person's work. Tried a few times: an app that
    /// was just started may not answer at once (`-25204`).
    pub(super) fn unminimize_parked(&mut self, window: &Window) -> Result<(), String> {
        let (Some(pid), Some(number)) = (
            window.pid.and_then(|pid| i32::try_from(pid).ok()),
            crate::assistant::computer::window_number(&window.id),
        ) else {
            return Err("the window has no process".into());
        };
        if cfg!(test) {
            return Ok(());
        }
        crate::assistant::actions::ax::wake(pid);
        let spot = super::displays::park_spot(&self.screen.displays());
        if spot.is_some() {
            self.note_parked(window, super::super::parking::How::OffScreen);
        }
        let mut last = 0;
        for attempt in 0..6 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(250 * attempt));
            }
            let listed = crate::assistant::actions::ax::windows(pid)
                .into_iter()
                .find(|listed| listed.number == number);
            match listed {
                Some(listed) if !listed.minimized => return Ok(()),
                Some(_) => {}
                // Not answering yet (an app just started): ask again.
                None => {
                    last = -25204;
                    continue;
                }
            }
            if let Some(spot) = spot {
                let _ = crate::assistant::actions::ax::set_position(pid, number, spot);
            }
            match crate::assistant::actions::ax::set_minimized(pid, number, false) {
                Ok(()) => {
                    std::thread::sleep(Duration::from_millis(400));
                    self.looked = None;
                    tracing::info!(
                        window = window.id,
                        attempt,
                        "restored a minimized window, parked"
                    );
                    return Ok(());
                }
                Err(code) => last = code,
            }
        }
        Err(format!("Accessibility couldn't restore it (error {last})"))
    }

    /// Right after a window was parked off the screen: whether it still
    /// gives a picture there. One that doesn't waits behind the person's
    /// windows instead (`decide::place`), on their display but not raised.
    pub(super) fn measure_parked(&mut self, window: &Window) {
        if cfg!(test) {
            return;
        }
        let Some(number) = crate::assistant::computer::window_number(&window.id) else {
            return;
        };
        std::thread::sleep(Duration::from_millis(150));
        let renders = shot::capture(number).is_ok_and(|(_, _, rgba)| {
            // A picture that is all transparent is no picture.
            rgba.as_chunks::<4>()
                .0
                .iter()
                .step_by(31)
                .any(|pixel| pixel[3] != 0)
        });
        let measured = decide::Measured {
            fresh_frame: Some(renders),
            input_took: None,
        };
        if decide::place(measured) == decide::Place::Behind {
            self.put_behind(window, "it gave no picture off the screen");
        }
    }

    /// Window `window` waits behind the person's windows from now on: moved
    /// onto their display without being raised, so it never covers their
    /// work but renders and takes input.
    pub(super) fn put_behind(&mut self, window: &Window, why: &str) {
        let displays = self.screen.displays();
        let Some(display) = displays
            .iter()
            .find(|display| display.has_focus)
            .or_else(|| displays.iter().find(|display| display.is_primary))
            .or(displays.first())
        else {
            return;
        };
        let spot = (display.frame.0 + 80.0, display.frame.1 + 80.0);
        self.note_parked(window, super::super::parking::How::Behind);
        self.move_window(window, spot);
        self.looked = None;
        tracing::info!(
            window = window.id,
            why,
            "a window waits behind the person's instead of off the screen"
        );
        self.notes.push(format!(
            "{} waits behind the person's windows now ({why}); act in it as before.",
            window.id
        ));
    }

    /// `hex_window_shot`.
    pub(super) fn window_shot(
        &mut self,
        tool: &str,
        args: &mut Map<String, Value>,
    ) -> Result<Value, Value> {
        let (_, mut window) = self.vision_window(tool, Kind::Observe, args)?;
        if window.minimized {
            self.unminimize_parked(&window).map_err(|why| {
                failure(&format!(
                    "Agent-Desk couldn't bring {} back: {why}",
                    window.id
                ))
            })?;
            window = self.window_info(&window.id).unwrap_or(window);
        }
        let number = crate::assistant::computer::window_number(&window.id)
            .ok_or_else(|| refusal(tool, "WINDOW_NOT_FOUND", "That isn't a window id (w-1234)."))?;
        let frame = self.frame_of(&window);
        self.shot_result(tool, &window, number, frame, None)
    }

    /// A picture of window `number` as a tool result: the image the agent
    /// sees (long side at most [`IMAGE_EDGE`]) and the full PNG's path.
    fn shot_result(
        &mut self,
        tool: &str,
        window: &Window,
        number: u32,
        frame: Option<Frame>,
        did: Option<Value>,
    ) -> Result<Value, Value> {
        if cfg!(test) {
            let image = frame.map_or((800, 600), |frame| frame.native_pixels());
            self.learned.vision.lock().shots.insert(
                number,
                Seen {
                    image,
                    full: image,
                    rgba: Vec::new(),
                },
            );
            return Ok(success(
                "window-shot",
                json!({"window": window.id, "image_width": image.0, "image_height": image.1, "did": did}),
            ));
        }
        let dir = self
            .store
            .root()
            .join("shots")
            .join(sanitize(&self.config.task));
        let before = self
            .learned
            .vision
            .lock()
            .shots
            .get(&number)
            .map(|seen| seen.rgba.clone());
        let (taken, rgba) = shot::take(number, &dir, before.as_deref()).map_err(|why| {
            failure(&format!(
                "Agent-Desk couldn't take {}'s picture: {why}",
                window.id
            ))
        })?;
        let (width, height, small) = shot::fit(&rgba, taken.width, taken.height, IMAGE_EDGE);
        let png = shot::encode_png(width, height, &small)
            .map_err(|why| failure(&format!("Agent-Desk couldn't encode the picture: {why}")))?;
        self.learned.vision.lock().shots.insert(
            number,
            Seen {
                image: (width, height),
                full: (taken.width, taken.height),
                rgba,
            },
        );
        let changed = taken.changed.map(|share| (share * 100.0).round());
        let mut result = success(
            if did.is_some() {
                "window-input"
            } else {
                "window-shot"
            },
            json!({
                "window": window.id,
                "app": window.app,
                "image_width": width,
                "image_height": height,
                "full_png": taken.path,
                "full_width": taken.width,
                "full_height": taken.height,
                "window_points": frame.map(|frame| [frame.width, frame.height]),
                "changed_percent": changed,
                "did": did,
                "note": "x,y for hex_window_input are pixels of this image (top-left 0,0). The \
                         window stays where it is; input goes to its app in the background.",
            }),
        );
        if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
            content.push(json!({
                "type": "image",
                "data": base64::engine::general_purpose::STANDARD.encode(png),
                "mimeType": "image/png",
            }));
        }
        let _ = tool;
        Ok(result)
    }

    /// `hex_window_input`.
    pub(super) fn window_input(
        &mut self,
        tool: &str,
        args: &mut Map<String, Value>,
    ) -> Result<Value, Value> {
        self.gate(tool)?;
        let action = text(args, "action").unwrap_or_else(|| "click".into());
        let (resolved, mut window) = self.vision_window(tool, Kind::Act, args)?;
        let number = crate::assistant::computer::window_number(&window.id)
            .ok_or_else(|| refusal(tool, "WINDOW_NOT_FOUND", "That isn't a window id (w-1234)."))?;
        if window.minimized {
            self.unminimize_parked(&window).map_err(|why| {
                failure(&format!(
                    "Agent-Desk couldn't bring {} back: {why}",
                    window.id
                ))
            })?;
            window = self.window_info(&window.id).unwrap_or(window);
        }
        let frame = self.frame_of(&window).ok_or_else(|| {
            failure("Agent-Desk can't tell where that window is; take a hex_window_shot first.")
        })?;
        let units = match (
            text(args, "units").as_deref(),
            self.learned.vision.lock().shots.get(&number),
        ) {
            (Some("points"), _) => Units::Points,
            (_, Some(seen)) => Units::ShotPixels {
                width: seen.image.0,
                height: seen.image.1,
            },
            // No picture yet: pixels of a shot at the display's resolution.
            (_, None) => {
                let (width, height) = frame.native_pixels();
                Units::ShotPixels { width, height }
            }
        };
        let number_of = |key: &str| args.get(key).and_then(Value::as_f64);
        let spot = match (number_of("x"), number_of("y")) {
            (Some(x), Some(y)) => Some(frame.target((x, y), units).ok_or_else(|| {
                refusal(
                    tool,
                    "OUT_OF_WINDOW",
                    "That spot isn't inside the window; use pixels of its last hex_window_shot.",
                )
            })?),
            _ => None,
        };
        let typed = args
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let gesture = match action.as_str() {
            "click" => Gesture::Click {
                button: Button::Left,
                count: 1,
            },
            "double_click" => Gesture::Click {
                button: Button::Left,
                count: 2,
            },
            "right_click" => Gesture::Click {
                button: Button::Right,
                count: 1,
            },
            "scroll" => {
                let amount = |key: &str| args.get(key).and_then(Value::as_f64).unwrap_or(0.0);
                let (dx, dy) = (amount("dx"), amount("dy"));
                let dy = if dx == 0.0 && dy == 0.0 { -300.0 } else { dy };
                Gesture::Scroll {
                    dx: dx.round() as i32,
                    dy: dy.round() as i32,
                }
            }
            "type" => Gesture::Text(typed.clone()),
            "key" => Gesture::Key(typed.clone()),
            other => {
                return Err(refusal(
                    tool,
                    "INVALID_ARGS",
                    &format!(
                        "Unknown action {other}; use click, double_click, right_click, scroll, \
                         type, or key."
                    ),
                ));
            }
        };
        if matches!(gesture, Gesture::Click { .. } | Gesture::Scroll { .. }) && spot.is_none() {
            return Err(refusal(
                tool,
                "INVALID_ARGS",
                "A click or scroll needs x and y (pixels of the window's last hex_window_shot).",
            ));
        }
        if matches!(&gesture, Gesture::Text(text) | Gesture::Key(text) if text.is_empty()) {
            return Err(refusal(
                tool,
                "INVALID_ARGS",
                "type needs text; key needs the combo in text (return, cmd+a, …).",
            ));
        }
        // What the action would do, asked as the guard asks of any action.
        let resolved = vec![resolved];
        match &gesture {
            Gesture::Click { .. } => {
                if let Some(spot) = spot {
                    let label = self.label_at(number, &frame, spot.local);
                    self.learned.elements.lock().insert(
                        SPOT_REF.into(),
                        super::consent::Element {
                            label,
                            ..super::consent::Element::default()
                        },
                    );
                    let mut ask = Map::new();
                    ask.insert("ref_id".into(), json!(SPOT_REF));
                    self.consent("desktop_click", &ask, &resolved)?;
                }
            }
            Gesture::Text(typed) => {
                let mut ask = Map::new();
                ask.insert("text".into(), json!(typed));
                self.consent("desktop_type", &ask, &resolved)?;
            }
            Gesture::Key(combo) => {
                let mut ask = Map::new();
                ask.insert("combo".into(), json!(combo));
                self.consent("desktop_press", &ask, &resolved)?;
            }
            Gesture::Scroll { .. } => {}
        }
        self.watch(&window.app);
        let _ = self.list(Look::Check);
        let pid = window
            .pid
            .and_then(|pid| i32::try_from(pid).ok())
            .ok_or_else(|| failure("Agent-Desk can't tell which process that window is."))?;
        // Text at a spot: click there first, so it lands in that field.
        if let (Gesture::Text(_), Some(spot)) = (&gesture, spot) {
            self.deliver(
                pid,
                number,
                &frame,
                Some(spot),
                &Gesture::Click {
                    button: Button::Left,
                    count: 1,
                },
            )?;
        }
        let delivered = self.deliver(pid, number, &frame, spot, &gesture)?;
        if matches!(gesture, Gesture::Text(_) | Gesture::Key(_)) {
            // Keys went by pid first: a focus for keys may follow (`borrow`).
            self.keys_tried.insert(window.id.clone());
        }
        let _ = self.list(Look::AfterAction);
        std::thread::sleep(Duration::from_millis(if cfg!(test) { 0 } else { 300 }));
        let did = json!({
            "action": action,
            "at_points": spot.map(|spot| [spot.local.0.round(), spot.local.1.round()]),
            "front_kept": delivered.front_before == delivered.front_after,
        });
        self.shot_result(tool, &window, number, Some(frame), Some(did))
    }

    /// Sends `gesture` to window `number` of `pid`, notes the spot for the
    /// live picture, and keeps the person's app in front.
    fn deliver(
        &mut self,
        pid: i32,
        number: u32,
        frame: &Frame,
        spot: Option<crate::assistant::input::geometry::Target>,
        gesture: &Gesture,
    ) -> Result<post::Delivered, Value> {
        if let Some(spot) = spot {
            pointer::note(number, frame.fraction(&spot));
        }
        #[cfg(test)]
        {
            let _ = (pid, frame);
            self.learned
                .vision
                .lock()
                .sent
                .push((number, gesture.clone()));
            Ok(post::Delivered::default())
        }
        #[cfg(not(test))]
        {
            let delivered = post::post(pid, number, frame, spot, gesture).map_err(|why| {
                failure(&format!(
                    "Agent-Desk couldn't send that to the window: {why}"
                ))
            })?;
            if let (Some(before), Some(after)) = (delivered.front_before, delivered.front_after)
                && before != after
                && after == pid
            {
                // The app took the front after all: the person gets it back.
                self.front.give_back(before);
                tracing::warn!(
                    pid,
                    "background input brought an app forward; gave the front back"
                );
            }
            Ok(delivered)
        }
    }

    /// The text at a spot of the window (its picture read on the device):
    /// the label of what a click there would press.
    fn label_at(&mut self, number: u32, frame: &Frame, (x, y): (f64, f64)) -> String {
        #[cfg(test)]
        {
            let _ = (number, frame, x, y);
            self.learned.vision.lock().label.clone()
        }
        #[cfg(not(test))]
        {
            crate::assistant::actions::ocr::read(number, Some((frame.width, frame.height)))
                .map(|reading| label_near(&reading.lines, (x, y)))
                .unwrap_or_default()
        }
    }

    /// After an acting call through Accessibility: when the app couldn't
    /// answer, once more, then by pid input at the element's spot. The
    /// result to keep.
    pub(super) fn ax_fallback(
        &mut self,
        tool: &str,
        args: &Map<String, Value>,
        resolved: &[Resolved],
        result: Value,
    ) -> Value {
        if decide::fallback(&result) != Fallback::Pointer {
            return result;
        }
        let name = tool.strip_prefix("desktop_").unwrap_or(tool);
        let pointable = matches!(
            name,
            "click"
                | "double_click"
                | "right_click"
                | "type"
                | "set_value"
                | "toggle"
                | "check"
                | "uncheck"
                | "expand"
                | "collapse"
                | "focus"
                | "select"
        );
        if !pointable {
            return result;
        }
        std::thread::sleep(Duration::from_millis(if cfg!(test) { 0 } else { 300 }));
        let retried = match self.forward(tool, args) {
            Ok(retried) if decide::fallback(&retried) == Fallback::None => return retried,
            Ok(retried) => retried,
            Err(_) => result,
        };
        let Some(reference) = text(args, "ref_id") else {
            return retried;
        };
        let Some(element) = self.learned.vision.lock().frames.get(&reference).copied() else {
            return retried;
        };
        let Some(id) = resolved.first().and_then(|window| window.id.clone()) else {
            return retried;
        };
        let Some(mut window) = self.window_info(&id) else {
            return retried;
        };
        if window.minimized && self.unminimize_parked(&window).is_ok() {
            window = self.window_info(&id).unwrap_or(window);
        }
        let (Some(frame), Some(number), Some(pid)) = (
            self.frame_of(&window),
            crate::assistant::computer::window_number(&id),
            window.pid.and_then(|pid| i32::try_from(pid).ok()),
        ) else {
            return retried;
        };
        let Some(spot) = frame.element_centre(element) else {
            return retried;
        };
        let value = args
            .get(if name == "type" { "text" } else { "value" })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let click = |count| Gesture::Click {
            button: Button::Left,
            count,
        };
        let steps: Vec<Gesture> = match name {
            "double_click" => vec![click(2)],
            "right_click" => vec![Gesture::Click {
                button: Button::Right,
                count: 1,
            }],
            "type" => vec![click(1), Gesture::Text(value)],
            "set_value" => vec![click(1), Gesture::Key("cmd+a".into()), Gesture::Text(value)],
            _ => vec![click(1)],
        };
        for step in &steps {
            let at = matches!(step, Gesture::Click { .. }).then_some(spot);
            if let Err(why) = self.deliver(pid, number, &frame, at, step) {
                tracing::warn!(?why, "the pointer fallback failed too");
                return retried;
            }
        }
        tracing::info!(
            tool,
            window = id,
            "Accessibility failed twice; acted by pid input"
        );
        success(
            &super::command(tool),
            json!({
                "fallback": "pid_input",
                "window": id,
                "at_points": [spot.local.0.round(), spot.local.1.round()],
                "note": "The app didn't answer through Accessibility, so Agent-Desk did this at the \
                         element's spot by input sent to the app in the background. Check the \
                         result with a fresh snapshot or hex_window_shot.",
            }),
        )
    }
}

/// The text at `(x, y)` among a window's read lines (points): the line
/// that holds the spot, else the nearest within a short reach.
fn label_near(lines: &[crate::assistant::actions::ocr::Line], (x, y): (f64, f64)) -> String {
    const REACH: f64 = 24.0;
    let distance = |line: &crate::assistant::actions::ocr::Line| {
        let dx = (line.x - x).max(0.0).max(x - (line.x + line.width));
        let dy = (line.y - y).max(0.0).max(y - (line.y + line.height));
        (dx * dx + dy * dy).sqrt()
    };
    lines
        .iter()
        .map(|line| (distance(line), line))
        .filter(|(distance, _)| *distance <= REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, line)| line.text.clone())
        .unwrap_or_default()
}

/// Every `(ref, frame)` in a result's data: elements with a ref and bounds
/// (`bounds`, Cua Driver's `frame`), in global points.
/// A frame: x, y, width, height in global points.
type Rect = (f64, f64, f64, f64);

fn frames_in(value: &Value, found: &mut Vec<(String, Rect)>) {
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("ref_id").and_then(Value::as_str)
                && let Some(frame) = fields.get("bounds").or_else(|| fields.get("frame"))
            {
                let number = |keys: &[&str]| keys.iter().find_map(|key| frame.get(*key)?.as_f64());
                if let (Some(x), Some(y), Some(width), Some(height)) = (
                    number(&["x"]),
                    number(&["y"]),
                    number(&["width", "w"]),
                    number(&["height", "h"]),
                ) {
                    found.push((reference.to_owned(), (x, y, width, height)));
                }
            }
            for field in fields.values() {
                frames_in(field, found);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| frames_in(item, found)),
        _ => {}
    }
}

fn sanitize(task: &str) -> String {
    task.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `hex_window_shot`'s schema.
pub(super) fn shot_tool() -> Value {
    json!({
        "name": WINDOW_SHOT_TOOL,
        "description": "Agent-Desk: a picture of one of your windows wherever it waits (off the screen, \
            behind the person's windows), as an image whose pixels are the coordinates \
            hex_window_input takes, plus the full-resolution PNG's path. For apps whose \
            Accessibility tree is missing or doesn't answer: look, then act by coordinates. \
            Never brings the window forward.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "window_id": {"type": "string", "description": "The window's id (w-1234)"},
                "app": {"type": "string", "description": "Or the app, when it has one window of yours"}
            },
        },
    })
}

/// `hex_window_input`'s schema.
pub(super) fn input_tool() -> Value {
    json!({
        "name": WINDOW_INPUT_TOOL,
        "description": "Agent-Desk: act in one of your windows at a spot of its last hex_window_shot, \
            by input sent to its app in the background: the person's pointer doesn't move and \
            their app keeps the front and the keyboard. action: click, double_click, \
            right_click, scroll (dx, dy in pixels; negative dy scrolls down), type (text; with \
            x,y it clicks there first), key (text is the combo: return, tab, cmd+a, …). Returns \
            a fresh picture and how much changed. Clicks on consequential controls (Send, Buy, \
            Delete) need the person's yes like any action.",
        "inputSchema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "window_id": {"type": "string", "description": "The window's id (w-1234)"},
                "action": {"type": "string", "enum": ["click", "double_click", "right_click", "scroll", "type", "key"]},
                "x": {"type": "number", "description": "Pixels from the left of the window's last hex_window_shot image"},
                "y": {"type": "number", "description": "Pixels from the top of that image"},
                "text": {"type": "string", "description": "What to type (type), or the key combo (key)"},
                "dx": {"type": "number", "description": "Horizontal scroll in pixels (scroll)"},
                "dy": {"type": "number", "description": "Vertical scroll in pixels; negative is down (scroll)"},
                "units": {"type": "string", "enum": ["shot", "points"], "description": "x,y in pixels of the last shot (default) or in points from the window's top-left (hex_read_window's)"}
            },
            "required": ["window_id", "action"],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::actions::ocr::Line;

    fn line(text: &str, x: f64, y: f64) -> Line {
        Line {
            text: text.into(),
            x,
            y,
            width: 60.0,
            height: 20.0,
            confidence: 1.0,
        }
    }

    #[test]
    fn a_spots_label_is_the_text_there_or_nearby() {
        let lines = [line("Abbrechen", 10.0, 10.0), line("Senden", 200.0, 10.0)];
        assert_eq!(label_near(&lines, (220.0, 20.0)), "Senden");
        assert_eq!(label_near(&lines, (275.0, 20.0)), "Senden");
        assert_eq!(label_near(&lines, (20.0, 25.0)), "Abbrechen");
        assert_eq!(label_near(&lines, (130.0, 200.0)), "");
    }

    #[test]
    fn element_frames_come_from_either_backend() {
        let data = json!({
            "tree": [{"ref_id": "@s1:e1", "bounds": {"x": 10.0, "y": 20.0, "width": 30.0, "height": 40.0}},
                     {"ref_id": "@s1:e2", "frame": {"x": 1.0, "y": 2.0, "w": 3.0, "h": 4.0}},
                     {"ref_id": "@s1:e3"}]
        });
        let mut found = Vec::new();
        frames_in(&data, &mut found);
        assert_eq!(
            found,
            [
                ("@s1:e1".into(), (10.0, 20.0, 30.0, 40.0)),
                ("@s1:e2".into(), (1.0, 2.0, 3.0, 4.0))
            ]
        );
    }
}
