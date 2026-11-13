//! What Agent-Desk itself plays (its own speech), kept for two seconds with when it
//! was handed to the output, so conversation mode can tell Agent-Desk's voice in
//! the microphone from the person's (`recognition::echo`). Only Agent-Desk's own
//! output is here; other apps' audio is never captured.
//!
//! The output thread pushes every 10 ms chunk it plays (silence included
//! while the device is open), so the timeline has no holes while Agent-Desk
//! reads; a query outside what was played gets zeros.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::instant::CaptureInstant;

/// How much is kept.
const KEEP: Duration = Duration::from_secs(2);
/// A chunk pushed within this of where the last one ended continues it
/// (the output pulls a little ahead of the clock); later, a new run starts.
const CONTINUE: Duration = Duration::from_millis(60);

struct Chunk {
    start: u64,
    samples: Vec<f32>,
}

struct Ring {
    rate: u32,
    chunks: VecDeque<Chunk>,
    /// Where the last chunk ends, in nanoseconds.
    end: u64,
    /// When something louder than silence last played.
    heard: Option<u64>,
}

static RING: Mutex<Option<Ring>> = Mutex::new(None);

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Keeps `samples` (mono at `rate`), handed to the output at `at`.
pub fn played(samples: &[f32], rate: u32, at: CaptureInstant) {
    if samples.is_empty() || rate == 0 {
        return;
    }
    // On the output's audio thread: never wait for the lock. A chunk
    // missed while a query reads only weakens one frame's judgement.
    let Ok(mut ring) = RING.try_lock() else {
        return;
    };
    push(&mut ring, samples, rate, at.as_nanos());
}

fn push(slot: &mut Option<Ring>, samples: &[f32], rate: u32, at: u64) {
    let ring = slot.get_or_insert_with(|| Ring {
        rate,
        chunks: VecDeque::new(),
        end: 0,
        heard: None,
    });
    if ring.rate != rate {
        *ring = Ring {
            rate,
            chunks: VecDeque::new(),
            end: 0,
            heard: None,
        };
    }
    let start = if ring.end > 0 && at <= ring.end.saturating_add(nanos(CONTINUE)) {
        ring.end
    } else {
        at
    };
    let length = samples.len() as u64 * 1_000_000_000 / u64::from(rate);
    ring.end = start + length;
    if samples.iter().any(|sample| sample.abs() > 1e-4) {
        ring.heard = Some(ring.end);
    }
    ring.chunks.push_back(Chunk {
        start,
        samples: samples.to_vec(),
    });
    let oldest = ring.end.saturating_sub(nanos(KEEP));
    while ring
        .chunks
        .front()
        .is_some_and(|chunk| chunk.start + nanos(KEEP) / 4 < oldest)
    {
        ring.chunks.pop_front();
    }
}

/// What Agent-Desk played from `end - length` to `end`, as `rate` mono samples
/// (`length * rate` of them), zeros where nothing played. `None` when Agent-Desk
/// played nothing audible in the 400 ms before `end`: there is no echo of
/// it to look for.
pub fn recent(end: CaptureInstant, length: Duration, rate: u32) -> Option<Vec<f32>> {
    let ring = RING.lock().unwrap_or_else(|error| error.into_inner());
    window(ring.as_ref()?, end.as_nanos(), length, rate)
}

fn window(ring: &Ring, end: u64, length: Duration, rate: u32) -> Option<Vec<f32>> {
    let heard = ring.heard?;
    if heard + nanos(Duration::from_millis(400)) < end.saturating_sub(nanos(length)) {
        return None;
    }
    let count = (length.as_secs_f64() * f64::from(rate)).round() as usize;
    let start = end.saturating_sub(nanos(length));
    let step = 1e9 / f64::from(rate);
    let source_step = 1e9 / f64::from(ring.rate);
    let mut out = vec![0.0; count];
    let mut chunks = ring.chunks.iter().peekable();
    for (index, slot) in out.iter_mut().enumerate() {
        let time = start as f64 + index as f64 * step;
        while let Some(chunk) = chunks.peek() {
            let chunk_end = chunk.start as f64 + chunk.samples.len() as f64 * source_step;
            if chunk_end <= time {
                chunks.next();
            } else {
                break;
            }
        }
        let Some(chunk) = chunks.peek() else {
            break;
        };
        let offset = (time - chunk.start as f64) / source_step;
        if offset < 0.0 {
            continue;
        }
        let at = offset as usize;
        let fraction = (offset - at as f64) as f32;
        let a = chunk.samples.get(at).copied().unwrap_or(0.0);
        let b = chunk.samples.get(at + 1).copied().unwrap_or(a);
        *slot = a + (b - a) * fraction;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn what_played_comes_back_at_another_rate() {
        let mut slot = None;
        // 100 ms of a ramp at 48 kHz, pushed in 10 ms chunks.
        for chunk in 0..10u64 {
            let samples: Vec<f32> = (0..480)
                .map(|i| (chunk * 480 + i) as f32 / 4_800.0)
                .collect();
            push(&mut slot, &samples, 48_000, 1_000 * MS + chunk * 10 * MS);
        }
        let ring = slot.as_ref().unwrap();
        let out = window(ring, 1_100 * MS, Duration::from_millis(20), 16_000).unwrap();
        assert_eq!(out.len(), 320);
        // 20 ms before the end: the ramp at 80 ms, 0.8.
        assert!((out[0] - 0.8).abs() < 0.01, "{}", out[0]);
        assert!(out[319] > 0.99);
    }

    #[test]
    fn nothing_played_lately_means_no_echo() {
        let mut slot = None;
        push(&mut slot, &[0.5; 480], 48_000, 1_000 * MS);
        let ring = slot.as_ref().unwrap();
        assert!(window(ring, 1_100 * MS, Duration::from_millis(20), 16_000).is_some());
        assert!(window(ring, 3_000 * MS, Duration::from_millis(20), 16_000).is_none());
        let mut silent = None;
        push(&mut silent, &[0.0; 480], 48_000, 1_000 * MS);
        assert!(
            window(
                silent.as_ref().unwrap(),
                1_010 * MS,
                Duration::from_millis(10),
                16_000
            )
            .is_none()
        );
    }

    #[test]
    fn a_window_before_or_after_the_played_audio_is_silent() {
        let mut slot = None;
        push(&mut slot, &[0.5; 480], 48_000, 1_000 * MS);
        let ring = slot.as_ref().unwrap();
        let out = window(ring, 1_015 * MS, Duration::from_millis(20), 16_000).unwrap();
        assert_eq!(out[0], 0.0, "before it played");
        assert!((out[200] - 0.5).abs() < 1e-6);
    }
}
