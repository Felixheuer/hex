//! Reading the WAV audio the engines write: 32-bit float (`say`) or 16-bit
//! PCM (Windows' voices, espeak-ng, Piper), at whatever rate they write,
//! brought to the mono [`RATE`] the output plays.

use super::playback::RATE;

/// A WAV's samples, its first channel, at [`RATE`]; `None` for anything
/// that isn't 32-bit float or 16-bit PCM. A streamed WAV (espeak-ng's
/// `--stdout`) gives its data an unknown size: what's there is read.
pub fn samples(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(at + 4)? as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " => {
                let tag = u16_at(body)?;
                let channels = u16_at(body + 2)?;
                let rate = u32_at(body + 4)?;
                let bits = u16_at(body + 14)?;
                format = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let (tag, channels, rate, bits) = format?;
                return decode(&bytes[body..end], tag, channels, rate, bits);
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// `data` in the format the `fmt ` chunk named, first channel, at [`RATE`].
/// Tag 1 is PCM, 3 IEEE float, 0xFFFE "extensible" (which `say` uses for
/// float and Windows for PCM; the bit depth tells them apart).
fn decode(data: &[u8], tag: u16, channels: u16, rate: u32, bits: u16) -> Option<Vec<f32>> {
    if channels == 0 || rate == 0 {
        return None;
    }
    let mono: Vec<f32> = match (tag, bits) {
        (3 | 0xFFFE, 32) => data
            .chunks_exact(4 * usize::from(channels))
            .map(|frame| f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]))
            .collect(),
        (1 | 0xFFFE, 16) => data
            .chunks_exact(2 * usize::from(channels))
            .map(|frame| f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32_768.0)
            .collect(),
        _ => return None,
    };
    Some(resample(&mono, rate, RATE))
}

/// `samples` at `from` Hz brought to `to` Hz by linear interpolation: plenty
/// for speech, which engines write at 16 or 22 kHz.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let count = (samples.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    (0..count)
        .map(|index| {
            let at = index as f64 * step;
            let left = at.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (at - left as f64) as f32;
            samples[left] * (1.0 - fraction) + samples[right] * fraction
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8], padding: bool) -> Vec<u8> {
        let block = bits / 8 * channels;
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        bytes.extend(b"fmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(tag.to_le_bytes());
        bytes.extend(channels.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * u32::from(block)).to_le_bytes());
        bytes.extend(block.to_le_bytes());
        bytes.extend(bits.to_le_bytes());
        if padding {
            bytes.extend(b"FLLR");
            bytes.extend(3_u32.to_le_bytes());
            bytes.extend([0, 0, 0, 0]);
        }
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    fn floats(samples: &[f32]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn says_float_wav_is_read() {
        let samples = [0.0, 0.25, -0.5, 1.0];
        assert_eq!(
            super::samples(&wav(3, 1, RATE, 32, &floats(&samples), true)).as_deref(),
            Some(&samples[..])
        );
        // Stereo: the first channel.
        assert_eq!(
            super::samples(&wav(0xFFFE, 2, RATE, 32, &floats(&samples), false)).as_deref(),
            Some(&[0.0, -0.5][..])
        );
        assert_eq!(super::samples(b"RIFF\0\0\0\0WAVEdata"), None);
        assert_eq!(super::samples(b"nonsense"), None);
        // 8-bit is nobody's speech.
        assert_eq!(super::samples(&wav(1, 1, RATE, 8, &[1, 2], false)), None);
    }

    #[test]
    fn pcm_at_another_rate_is_brought_to_the_output_rate() {
        let pcm: Vec<u8> = [0_i16, 16_384, -32_768, 32_767]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let read = super::samples(&wav(1, 1, RATE, 16, &pcm, false)).unwrap();
        assert_eq!(read, [0.0, 0.5, -1.0, 32_767.0 / 32_768.0]);
        // 16 kHz, as Windows' voices write: three times as many samples.
        let read = super::samples(&wav(1, 1, 16_000, 16, &pcm, false)).unwrap();
        assert_eq!(read.len(), 12);
        assert_eq!(read[0], 0.0);
        assert_eq!(read[3], 0.5);
        assert!((read[1] - 0.5 / 3.0).abs() < 1e-6);
    }

    /// espeak-ng's `--stdout` streams: its sizes say "as much as comes".
    #[test]
    fn a_streamed_wav_reads_what_arrived() {
        let pcm: Vec<u8> = [8_192_i16, 8_192]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let mut bytes = wav(1, 1, RATE, 16, &pcm, false);
        let data_size = bytes.len() - pcm.len() - 4;
        bytes[data_size..data_size + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(super::samples(&bytes), Some(vec![0.25, 0.25]));
    }

    #[test]
    fn resampling_keeps_the_shape() {
        assert_eq!(resample(&[1.0, 2.0], 48_000, 48_000), [1.0, 2.0]);
        assert_eq!(resample(&[0.0, 1.0], 24_000, 48_000), [0.0, 0.5, 1.0, 1.0]);
        assert_eq!(resample(&[0.0, 1.0, 0.0, 1.0], 96_000, 48_000), [0.0, 0.0]);
        assert!(resample(&[], 16_000, 48_000).is_empty());
    }
}
