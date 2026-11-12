//! Barge-in thresholds that learn this Mac's rooms. Talking over an answer
//! pauses it once the person's voice holds for a number of 20 ms frames
//! (`recognition::conversation`); too few and Hex cuts itself off on its own
//! echo, too many and the person has to shout over it. The defaults are
//! guesses from synthetic signals, so every barge-in's outcome tunes them:
//!
//! - a **false alarm** (the capture held nothing, or only Hex's own words:
//!   the reading resumed) asks for more voice next time on that output;
//! - a **take-over** (the person's own words, or a spoken stop) allows a
//!   little less.
//!
//! The steps are lopsided ([`FALSE_ALARM_STEP`] up, [`TAKE_OVER_STEP`]
//! down), so the thresholds settle where about one barge-in in ten is a
//! false alarm, and stay within [`Thresholds::BOUNDS`]. Headphones and
//! speakers learn apart. The thresholds and the last decisions are kept in
//! `<assistant dir>/barge-in.json`; `hex assistant barge-in` shows them and
//! `--reset` forgets them. Every decision is logged as `barge-in: learned`.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// Where the answer is heard.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Headphones or AirPods: the microphone can't hear Hex's voice.
    Headphones,
    /// Speakers, or anything Hex can't tell.
    Speakers,
}

/// How a barge-in turned out.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing, or Hex's own voice: the reading resumed.
    FalseAlarm,
    /// The person's words: they asked something new.
    TookOver,
    /// A spoken stop.
    Stopped,
}

/// The voice a barge-in needs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Speech frames in a run on headphones (no echo).
    pub headphone_frames: f32,
    /// Speech frames in a run on speakers, judged against the echo.
    pub speaker_frames: f32,
    /// On speakers before the echo path is learnt: how far above the
    /// answer's playback level a frame must be, in dB.
    pub speaker_margin_db: f32,
}

impl Thresholds {
    /// The starting point: 240 ms, 500 ms, 6 dB.
    pub const DEFAULT: Self = Self {
        headphone_frames: 12.0,
        speaker_frames: 25.0,
        speaker_margin_db: 6.0,
    };

    /// The least and most each may become.
    pub const BOUNDS: [(f32, f32); 3] = [(8.0, 30.0), (15.0, 60.0), (3.0, 15.0)];

    /// Frames a run needs on `route`, whole.
    pub fn frames(&self, route: Route) -> u32 {
        let frames = match route {
            Route::Headphones => self.headphone_frames,
            Route::Speakers => self.speaker_frames,
        };
        frames.round().max(1.0) as u32
    }

    fn clamped(self) -> Self {
        let [h, s, m] = Self::BOUNDS;
        let fix = |value: f32, (low, high): (f32, f32), default: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                default
            }
        };
        Self {
            headphone_frames: fix(self.headphone_frames, h, Self::DEFAULT.headphone_frames),
            speaker_frames: fix(self.speaker_frames, s, Self::DEFAULT.speaker_frames),
            speaker_margin_db: fix(self.speaker_margin_db, m, Self::DEFAULT.speaker_margin_db),
        }
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How far a false alarm raises a route's frames (and, on speakers, the
/// margin in dB).
const FALSE_ALARM_STEP: (f32, f32) = (3.0, 1.0);
/// How far a take-over lowers them: a ninth of the rise, so about one
/// barge-in in ten may be a false alarm before they climb.
const TAKE_OVER_STEP: (f32, f32) = (1.0 / 3.0, 1.0 / 9.0);

/// The thresholds after a barge-in on `route` turned out as `outcome`.
pub fn learn(now: Thresholds, route: Route, outcome: Outcome) -> Thresholds {
    let (frames, db) = match outcome {
        Outcome::FalseAlarm => FALSE_ALARM_STEP,
        Outcome::TookOver | Outcome::Stopped => (-TAKE_OVER_STEP.0, -TAKE_OVER_STEP.1),
    };
    let mut next = now;
    match route {
        Route::Headphones => next.headphone_frames += frames,
        Route::Speakers => {
            next.speaker_frames += frames;
            next.speaker_margin_db += db;
        }
    }
    next.clamped()
}

/// One barge-in, as remembered for `hex assistant barge-in`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    /// Unix milliseconds.
    pub at_ms: u64,
    pub route: Route,
    /// The frames it needed, and whether the echo judge decided (speakers
    /// with a learnt echo path) or the margin over the playback level.
    pub needed: u32,
    pub echo_judged: bool,
    /// The microphone's and the answer's level when it fired, in dB.
    pub level_db: Option<f32>,
    pub playback_db: Option<f32>,
    pub outcome: Outcome,
}

/// What a barge-in looked like when it fired, until its outcome is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fired {
    pub route: Route,
    pub needed: u32,
    pub echo_judged: bool,
    pub level_db: Option<f32>,
    pub playback_db: Option<f32>,
}

/// The decisions kept.
const KEPT: usize = 50;
/// A fired barge-in whose outcome takes longer than this is forgotten.
const SETTLE_WITHIN_MS: u64 = 120_000;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    thresholds: Thresholds,
    #[serde(default)]
    decisions: VecDeque<Decision>,
}

#[derive(Default)]
struct State {
    saved: Saved,
    loaded: bool,
    fired: Option<(u64, Fired)>,
}

fn state() -> std::sync::MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    let mut state = STATE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !state.loaded {
        state.loaded = true;
        state.saved = load().unwrap_or_default();
        state.saved.thresholds = state.saved.thresholds.clamped();
    }
    state
}

fn path() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::assistant::store::assistant_dir()
        .ok()
        .map(|dir| dir.join("barge-in.json"))
}

fn load() -> Option<Saved> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text)
        .map_err(|error| tracing::warn!(%error, "barge-in.json is unreadable; starting over"))
        .ok()
}

fn save(saved: &Saved) {
    let Some(path) = path() else {
        return;
    };
    let result = serde_json::to_vec_pretty(saved)
        .map_err(std::io::Error::other)
        .and_then(|bytes| crate::durable::write(&path, &bytes));
    if let Err(error) = result {
        tracing::warn!(%error, "couldn't save the barge-in thresholds");
    }
}

/// The thresholds a barge-in needs now.
pub fn thresholds() -> Thresholds {
    state().saved.thresholds
}

/// A barge-in fired: its outcome comes with [`settle`].
pub fn fired(fired: Fired) {
    state().fired = Some((crate::assistant::store::now_ms(), fired));
}

/// The last barge-in turned out as `outcome`: the thresholds learn from it.
/// Nothing when none fired lately (a hold, not a barge-in).
pub fn settle(outcome: Outcome) {
    let now = crate::assistant::store::now_ms();
    let mut state = state();
    let Some((at, fired)) = state.fired.take() else {
        return;
    };
    if now.saturating_sub(at) > SETTLE_WITHIN_MS {
        return;
    }
    let before = state.saved.thresholds;
    let after = learn(before, fired.route, outcome);
    tracing::info!(
        route = ?fired.route,
        needed = fired.needed,
        echo_judged = fired.echo_judged,
        level_db = ?fired.level_db,
        playback_db = ?fired.playback_db,
        ?outcome,
        frames_now = after.frames(fired.route),
        margin_db_now = after.speaker_margin_db,
        "barge-in: learned"
    );
    state.saved.thresholds = after;
    state.saved.decisions.push_back(Decision {
        at_ms: at,
        route: fired.route,
        needed: fired.needed,
        echo_judged: fired.echo_judged,
        level_db: fired.level_db,
        playback_db: fired.playback_db,
        outcome,
    });
    while state.saved.decisions.len() > KEPT {
        state.saved.decisions.pop_front();
    }
    save(&state.saved);
}

/// What `hex assistant barge-in` prints: the thresholds and how the last
/// barge-ins turned out.
pub fn report() -> String {
    let state = state();
    let t = state.saved.thresholds;
    let mut out = format!(
        "Barge-in thresholds (learnt on this Mac)\n  headphones: {} ms of voice\n  speakers:   {} ms of voice over Hex's echo, or {:.1} dB over the answer before the echo is learnt\n",
        t.frames(Route::Headphones) * 20,
        t.frames(Route::Speakers) * 20,
        t.speaker_margin_db,
    );
    let decisions = &state.saved.decisions;
    if decisions.is_empty() {
        out.push_str("No barge-ins yet.\n");
        return out;
    }
    for route in [Route::Headphones, Route::Speakers] {
        let on: Vec<&Decision> = decisions.iter().filter(|d| d.route == route).collect();
        if on.is_empty() {
            continue;
        }
        let false_alarms = on
            .iter()
            .filter(|d| d.outcome == Outcome::FalseAlarm)
            .count();
        out.push_str(&format!(
            "  {route:?}: {} barge-ins, {false_alarms} false alarms\n",
            on.len()
        ));
    }
    out.push_str("Last barge-ins (newest last):\n");
    for d in decisions.iter().rev().take(10).rev() {
        out.push_str(&format!(
            "  {:?} on {:?}: needed {} ms ({}), level {} dB, answer {} dB\n",
            d.outcome,
            d.route,
            d.needed * 20,
            if d.echo_judged { "echo judged" } else { "margin" },
            d.level_db.map_or("?".into(), |db| format!("{db:.0}")),
            d.playback_db.map_or("?".into(), |db| format!("{db:.0}")),
        ));
    }
    out
}

/// Forgets what was learnt: back to the defaults.
pub fn reset() {
    let mut state = state();
    state.saved = Saved::default();
    state.fired = None;
    save(&state.saved);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_false_alarm_asks_for_more_voice_and_a_take_over_for_a_little_less() {
        let start = Thresholds::DEFAULT;
        let up = learn(start, Route::Speakers, Outcome::FalseAlarm);
        assert_eq!(up.frames(Route::Speakers), 28);
        assert_eq!(up.speaker_margin_db, 7.0);
        assert_eq!(up.headphone_frames, start.headphone_frames, "routes learn apart");
        let down = learn(up, Route::Speakers, Outcome::TookOver);
        assert!(down.speaker_frames < up.speaker_frames);
        assert!(down.speaker_frames > start.speaker_frames);
        let phones = learn(start, Route::Headphones, Outcome::Stopped);
        assert!(phones.headphone_frames < start.headphone_frames);
        assert_eq!(phones.speaker_frames, start.speaker_frames);
    }

    #[test]
    fn they_settle_near_one_false_alarm_in_ten_and_stay_in_bounds() {
        // Nine take-overs and one false alarm, over and over: steady.
        let mut t = Thresholds::DEFAULT;
        for _ in 0..200 {
            for _ in 0..9 {
                t = learn(t, Route::Speakers, Outcome::TookOver);
            }
            t = learn(t, Route::Speakers, Outcome::FalseAlarm);
        }
        assert!((t.speaker_frames - Thresholds::DEFAULT.speaker_frames).abs() < 3.5);
        // Only false alarms: up to the ceiling, never past it.
        for _ in 0..100 {
            t = learn(t, Route::Speakers, Outcome::FalseAlarm);
        }
        assert_eq!(t.frames(Route::Speakers), 60);
        assert_eq!(t.speaker_margin_db, 15.0);
        // Only take-overs: down to the floor.
        for _ in 0..1000 {
            t = learn(t, Route::Speakers, Outcome::TookOver);
            t = learn(t, Route::Headphones, Outcome::TookOver);
        }
        assert_eq!(t.frames(Route::Speakers), 15);
        assert_eq!(t.frames(Route::Headphones), 8);
        assert_eq!(t.speaker_margin_db, 3.0);
    }

    #[test]
    fn a_broken_file_falls_back_to_the_defaults() {
        let t = Thresholds {
            headphone_frames: f32::NAN,
            speaker_frames: 1_000.0,
            speaker_margin_db: -4.0,
        }
        .clamped();
        assert_eq!(t.headphone_frames, Thresholds::DEFAULT.headphone_frames);
        assert_eq!(t.speaker_frames, 60.0);
        assert_eq!(t.speaker_margin_db, 3.0);
        let saved: Saved = serde_json::from_str("{}").unwrap();
        assert_eq!(saved.thresholds, Thresholds::DEFAULT);
    }

    #[test]
    fn only_a_barge_in_that_fired_learns() {
        reset();
        settle(Outcome::FalseAlarm);
        assert_eq!(thresholds(), Thresholds::DEFAULT, "a hold is no barge-in");
        fired(Fired {
            route: Route::Speakers,
            needed: 25,
            echo_judged: true,
            level_db: Some(-30.0),
            playback_db: Some(-28.0),
        });
        settle(Outcome::FalseAlarm);
        assert_eq!(thresholds().frames(Route::Speakers), 28);
        settle(Outcome::FalseAlarm);
        assert_eq!(thresholds().frames(Route::Speakers), 28, "once per barge-in");
        assert!(report().contains("FalseAlarm on Speakers"));
        reset();
    }
}
