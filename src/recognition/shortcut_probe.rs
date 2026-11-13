//! "Press your shortcut now": whether a shortcut's press and release
//! actually reach Agent-Desk's event tap, with exactly its keys and sides, before
//! another app or macOS takes them (macOS).
//!
//! A test runs on the listener's thread: while it does, every key and
//! modifier edge goes to it instead of the shortcut machines (they only keep
//! their key state), so it starts no recording and asks nothing. The HID tap
//! reports every key press meanwhile (`suppression::set_probing`), so a key
//! that reached the keyboard but never the session tap shows another app or
//! macOS took it first.
//!
//! ```ts
//! start(slot, wait) -> Receiver<ProbeResult>   // from any thread; one test at a time
//! ├── listener not running, shortcuts paused, or no shortcut -> Unavailable at once
//! ├── a recording runs -> Unavailable
//! ├── its press with exactly its keys, then its release -> Works { held }
//! ├── the press but no release within 10 s -> NoRelease
//! ├── its key at the keyboard, never at the tap -> Swallowed
//! ├── its keys with other modifiers or sides -> WrongKeys { saw }
//! └── nothing within `wait` -> NothingReceived
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::app_settings::{HOTKEY_MODIFIERS_MASK, HotkeyBinding, HotkeySlot, RuntimeHotkey};
use crate::audio::CaptureInstant;
use crate::suppression::InputEvent;

/// How long a press may be held before its release counts as missing.
const RELEASE_WAIT: Duration = Duration::from_secs(10);

/// What a shortcut test found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProbeResult {
    /// The press and its release reached Agent-Desk with exactly the shortcut's
    /// keys; `held` is how long it was down.
    Works { held: Duration },
    /// The press reached Agent-Desk, but its release didn't come within 10 s.
    NoRelease,
    /// The keyboard reported the shortcut's key, but Agent-Desk's tap never got it:
    /// another app's shortcut or macOS took it first.
    Swallowed,
    /// The shortcut's keys arrived with other modifiers or on the other
    /// side; `saw` is what was held, as keycaps ("L⌘").
    WrongKeys { saw: String },
    /// Nothing of the shortcut reached Agent-Desk in time: another app or macOS
    /// took it, or it wasn't pressed.
    NothingReceived,
    /// The test couldn't run; the text is for a person.
    Unavailable(String),
}

/// Tests `slot`'s shortcut as the listener matches it now: the reply comes
/// once it is pressed and released, or after `wait` without a press. A test
/// already running ends `Unavailable`.
// Called by the assistant setup's "Shortcut works" step.
#[allow(dead_code)]
pub(crate) fn start(slot: HotkeySlot, wait: Duration) -> Receiver<ProbeResult> {
    let hotkeys = crate::app_settings::runtime_hotkeys();
    let binding = match slot {
        HotkeySlot::Dictation => Some(hotkeys.dictation),
        HotkeySlot::Action => hotkeys.action,
        HotkeySlot::StopAll => hotkeys.stop_all,
        HotkeySlot::SecondDictation => hotkeys.second_dictation,
        HotkeySlot::RawDictation => hotkeys.raw_dictation,
        HotkeySlot::PasteLast => hotkeys.paste_last,
        _ => None,
    };
    let (reply, result) = sync_channel(1);
    match binding {
        Some(binding) => start_with(binding, wait, reply),
        None => {
            let _ = reply.send(ProbeResult::Unavailable(
                "This shortcut isn't on: record one first".into(),
            ));
        }
    }
    result
}

/// Tests `binding` as [`start`] does; for a shortcut not saved yet.
// Called by the assistant setup's "Shortcut works" step.
#[allow(dead_code)]
pub(crate) fn start_binding(binding: &HotkeyBinding, wait: Duration) -> Receiver<ProbeResult> {
    let (reply, result) = sync_channel(1);
    start_with(binding.runtime(), wait, reply);
    result
}

/// Ends a running test without an answer (its receiver disconnects).
// Called by the assistant setup's "Shortcut works" step.
#[allow(dead_code)]
pub(crate) fn cancel() {
    if probe().take().is_some() {
        crate::suppression::set_probing(false);
    }
}

/// Whether a test is waiting for a press.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn active() -> bool {
    probe().is_some()
}

fn start_with(binding: RuntimeHotkey, wait: Duration, reply: SyncSender<ProbeResult>) {
    let refusal = if !LISTENING.load(Ordering::Acquire) {
        Some("Agent-Desk isn't listening for shortcuts yet")
    } else if crate::app_settings::shortcuts_suspended() {
        Some("Agent-Desk is recording a shortcut in Settings; finish that first")
    } else if binding.modifiers.is_empty() && binding.key_code.is_none() {
        Some("This shortcut isn't on: record one first")
    } else {
        None
    };
    if let Some(why) = refusal {
        let _ = reply.send(ProbeResult::Unavailable(why.into()));
        return;
    }
    let previous = probe().replace(Probe {
        matcher: Matcher::new(binding),
        deadline: Instant::now() + wait,
        reply,
    });
    if let Some(previous) = previous {
        let _ = previous.reply.send(ProbeResult::Unavailable(
            "Another shortcut test started".into(),
        ));
    }
    crate::suppression::set_probing(true);
}

struct Probe {
    matcher: Matcher,
    /// When it gives up: `wait` after the start, or 10 s after the press.
    deadline: Instant,
    reply: SyncSender<ProbeResult>,
}

static PROBE: Mutex<Option<Probe>> = Mutex::new(None);
static LISTENING: AtomicBool = AtomicBool::new(false);

fn probe() -> MutexGuard<'static, Option<Probe>> {
    PROBE.lock().unwrap_or_else(|error| error.into_inner())
}

fn finish(slot: &mut Option<Probe>, result: ProbeResult) {
    if let Some(probe) = slot.take() {
        crate::suppression::set_probing(false);
        let _ = probe.reply.send(result);
    }
}

/// The listener runs while this lives; a test needs it.
pub(super) struct Listening;

impl Listening {
    pub(super) fn start() -> Self {
        LISTENING.store(true, Ordering::Release);
        Self
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        LISTENING.store(false, Ordering::Release);
        finish(
            &mut probe(),
            ProbeResult::Unavailable("Agent-Desk stopped listening for shortcuts".into()),
        );
    }
}

/// The listener's input edge: whether a test took it (the shortcut machines
/// then only track its key state).
pub(super) fn observe(event: InputEvent, at: CaptureInstant) -> bool {
    let mut slot = probe();
    let Some(probe) = slot.as_mut() else {
        return false;
    };
    match probe.matcher.feed(event, at) {
        Some(result) => finish(&mut slot, result),
        None if probe.matcher.pressed() => {
            probe.deadline = probe.deadline.max(Instant::now() + RELEASE_WAIT);
        }
        None => {}
    }
    true
}

/// Each listener pass: a test ends once its time is up, or at once while a
/// recording runs (`busy`).
pub(super) fn tick(busy: bool) {
    let mut slot = probe();
    let Some(probe) = slot.as_ref() else {
        return;
    };
    if busy {
        finish(
            &mut slot,
            ProbeResult::Unavailable("A recording is in progress; finish it first".into()),
        );
    } else if Instant::now() >= probe.deadline {
        let result = probe.matcher.timed_out();
        finish(&mut slot, result);
    }
}

/// Follows the edges of one shortcut test.
#[derive(Debug)]
struct Matcher {
    binding: RuntimeHotkey,
    pressed_at: Option<CaptureInstant>,
    /// The keyboard reported the shortcut's key (HID tap).
    physical: bool,
    /// The shortcut's keys with the wrong modifiers: the flags held.
    near: Option<u64>,
}

impl Matcher {
    fn new(binding: RuntimeHotkey) -> Self {
        Self {
            binding,
            pressed_at: None,
            physical: false,
            near: None,
        }
    }

    fn pressed(&self) -> bool {
        self.pressed_at.is_some()
    }

    /// The result `event` settles, if it settles one.
    fn feed(&mut self, event: InputEvent, at: CaptureInstant) -> Option<ProbeResult> {
        let binding = self.binding;
        let released = |flags: u64| !binding.required_modifiers_down(flags);
        let works = |pressed: CaptureInstant| ProbeResult::Works {
            held: at.checked_duration_since(pressed).unwrap_or_default(),
        };
        match (event, binding.key_code, self.pressed_at) {
            (InputEvent::TapDisabled, ..) => Some(ProbeResult::Unavailable(
                "Agent-Desk's keyboard access was interrupted; try again".into(),
            )),
            // A shortcut of modifiers alone: exactly them down, then up.
            (InputEvent::Flags(flags), None, None) => {
                if binding.exact_modifiers(flags) {
                    self.pressed_at = Some(at);
                } else if binding.required_modifiers_down(flags)
                    || (flags & HOTKEY_MODIFIERS_MASK != 0 && self.shares_a_modifier(flags))
                {
                    self.near = Some(flags);
                }
                None
            }
            (InputEvent::Flags(flags), _, Some(pressed)) if released(flags) => Some(works(pressed)),
            // A shortcut with a key: the key down with exactly its modifiers,
            // then the key or a modifier up.
            (
                InputEvent::Key {
                    code,
                    down: true,
                    flags,
                },
                Some(key),
                None,
            ) if code == key => {
                if binding.exact_modifiers(flags) {
                    self.pressed_at = Some(at);
                } else {
                    self.near = Some(flags);
                }
                None
            }
            (
                InputEvent::Key {
                    code, down: false, ..
                },
                Some(key),
                Some(pressed),
            ) if code == key => Some(works(pressed)),
            (InputEvent::PhysicalKeyDown { code, .. }, Some(key), _) if code == key => {
                self.physical = true;
                None
            }
            _ => None,
        }
    }

    /// The answer once time is up.
    fn timed_out(&self) -> ProbeResult {
        if self.pressed() {
            ProbeResult::NoRelease
        } else if let Some(flags) = self.near {
            let modifiers = crate::app_settings::modifiers_from_flags(flags);
            let mut saw = HotkeyBinding {
                modifiers,
                key: None,
            }
            .keycaps()
            .join(" ");
            if self.binding.key_code.is_some() {
                saw = if saw.is_empty() {
                    "its key alone".into()
                } else {
                    format!("{saw} with its key")
                };
            }
            ProbeResult::WrongKeys { saw }
        } else if self.physical {
            ProbeResult::Swallowed
        } else {
            ProbeResult::NothingReceived
        }
    }

    /// Whether `flags` hold a modifier the shortcut names, on any side.
    fn shares_a_modifier(&self, flags: u64) -> bool {
        let held = crate::app_settings::modifiers_from_flags(flags);
        let m = self.binding.modifiers;
        (m.command.is_some() && held.command.is_some())
            || (m.option.is_some() && held.option.is_some())
            || (m.control.is_some() && held.control.is_some())
            || (m.shift.is_some() && held.shift.is_some())
            || (m.function && held.function)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::{
        COMMAND_KEY_MASK, LEFT_COMMAND_MASK, OPTION_KEY_MASK, RIGHT_COMMAND_MASK,
    };
    use crate::suppression::{HidClock, HidTiming};

    const RIGHT_COMMAND: u64 = COMMAND_KEY_MASK | RIGHT_COMMAND_MASK;
    const LEFT_COMMAND: u64 = COMMAND_KEY_MASK | LEFT_COMMAND_MASK;

    fn at(ms: u64) -> CaptureInstant {
        CaptureInstant::ZERO + Duration::from_secs(60) + Duration::from_millis(ms)
    }

    fn right_command() -> Matcher {
        Matcher::new(HotkeyBinding::action_default().runtime())
    }

    fn option_space() -> Matcher {
        Matcher::new(RuntimeHotkey {
            modifiers: crate::app_settings::HotkeyModifiers::option(),
            key_code: Some(49),
        })
    }

    #[test]
    fn a_press_and_release_with_exactly_its_keys_works() {
        let mut probe = right_command();
        assert_eq!(probe.feed(InputEvent::Flags(RIGHT_COMMAND), at(0)), None);
        assert_eq!(
            probe.feed(InputEvent::Flags(0), at(400)),
            Some(ProbeResult::Works {
                held: Duration::from_millis(400)
            })
        );

        let mut probe = option_space();
        let space = |down: bool| InputEvent::Key {
            code: 49,
            down,
            flags: OPTION_KEY_MASK,
        };
        assert_eq!(probe.feed(InputEvent::Flags(OPTION_KEY_MASK), at(0)), None);
        assert_eq!(probe.feed(space(true), at(50)), None);
        assert_eq!(
            probe.feed(space(false), at(150)),
            Some(ProbeResult::Works {
                held: Duration::from_millis(100)
            })
        );
    }

    #[test]
    fn the_other_side_or_another_modifier_is_the_wrong_keys() {
        let mut probe = right_command();
        assert_eq!(probe.feed(InputEvent::Flags(LEFT_COMMAND), at(0)), None);
        assert_eq!(probe.feed(InputEvent::Flags(0), at(200)), None);
        assert!(matches!(probe.timed_out(), ProbeResult::WrongKeys { saw } if saw.contains('⌘')));

        let mut probe = option_space();
        let command_space = InputEvent::Key {
            code: 49,
            down: true,
            flags: COMMAND_KEY_MASK,
        };
        assert_eq!(probe.feed(command_space, at(0)), None);
        assert!(matches!(probe.timed_out(), ProbeResult::WrongKeys { .. }));
    }

    /// The keyboard saw the key but the session tap never did: someone took
    /// it first. Nothing at all is nothing received; a press without its
    /// release is a missing release.
    #[test]
    fn a_key_taken_before_the_tap_is_swallowed() {
        let mut probe = option_space();
        let physical = InputEvent::PhysicalKeyDown {
            code: 49,
            timing: HidTiming {
                raw: 0,
                clock: HidClock::Arrival,
                received_at: at(0),
            },
        };
        assert_eq!(probe.feed(physical, at(0)), None);
        assert_eq!(probe.timed_out(), ProbeResult::Swallowed);

        assert_eq!(option_space().timed_out(), ProbeResult::NothingReceived);
        let mut probe = right_command();
        probe.feed(InputEvent::Flags(RIGHT_COMMAND), at(0));
        assert_eq!(probe.timed_out(), ProbeResult::NoRelease);
        assert!(matches!(
            probe.feed(InputEvent::TapDisabled, at(10)),
            Some(ProbeResult::Unavailable(_))
        ));
    }

    /// Without the listener running a test answers at once, and a test
    /// never outlives the listener.
    #[test]
    fn a_test_needs_the_listener() {
        let reply = start_binding(&HotkeyBinding::action_default(), Duration::from_secs(5));
        assert!(matches!(
            reply.recv_timeout(Duration::from_millis(100)),
            Ok(ProbeResult::Unavailable(_))
        ));
        assert!(!active());
    }
}
