//! Supertonic 3 on ONNX Runtime: a sentence in, 44.1 kHz mono audio out.
//!
//! Four networks run in turn: the duration predictor (how long the sentence
//! lasts), the text encoder, the vector estimator (a few denoising steps
//! from noise to the sentence's latent), and the vocoder (latent to audio).
//!
//! Ported from Supertone's MIT-licensed Rust example
//! (`rust/src/helper.rs`, `TextToSpeech::_infer` and `load_voice_style`):
//! Copyright (c) 2025 Supertone Inc., MIT License (full text in
//! THIRD_PARTY_NOTICES.md). Tensors go in as shape and data instead of
//! ndarray arrays; the arithmetic is upstream's.

use std::path::Path;

use color_eyre::eyre::{Result, WrapErr, eyre};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;

use super::text;

/// Denoising steps per sentence. Upstream's examples use 8; each step is
/// most of the time a sentence takes (about 45 ms per 2 s of speech on an
/// M-series performance core set), and 5 reads at a tenth of real time.
pub const STEPS: usize = 5;
/// Speed: upstream's default (`--speed 1.05`).
pub const SPEED: f32 = 1.05;

/// `tts.json`'s numbers the inference needs.
#[derive(Clone, Debug, Deserialize)]
struct Config {
    ae: AutoEncoder,
    ttl: TextToLatent,
}

#[derive(Clone, Debug, Deserialize)]
struct AutoEncoder {
    sample_rate: u32,
    base_chunk_size: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct TextToLatent {
    chunk_compress_factor: usize,
    latent_dim: usize,
}

/// A preset voice (`voice_styles/M1.json`): its two style tensors.
pub struct Style {
    ttl: (Vec<usize>, Vec<f32>),
    dp: (Vec<usize>, Vec<f32>),
}

#[derive(Deserialize)]
struct StyleFile {
    style_ttl: StyleTensor,
    style_dp: StyleTensor,
}

#[derive(Deserialize)]
struct StyleTensor {
    data: Vec<Vec<Vec<f32>>>,
    dims: Vec<usize>,
}

impl StyleTensor {
    fn flatten(self) -> Result<(Vec<usize>, Vec<f32>)> {
        let data: Vec<f32> = self.data.into_iter().flatten().flatten().collect();
        let expected: usize = self.dims.iter().product();
        if self.dims.len() != 3 || data.len() != expected {
            return Err(eyre!(
                "a voice style has {} values for dims {:?}",
                data.len(),
                self.dims
            ));
        }
        Ok((self.dims, data))
    }
}

impl Style {
    pub fn load(path: &Path) -> Result<Self> {
        let file: StyleFile = serde_json::from_slice(&std::fs::read(path)?)
            .wrap_err_with(|| format!("reading {}", path.display()))?;
        Ok(Self {
            ttl: file.style_ttl.flatten()?,
            dp: file.style_dp.flatten()?,
        })
    }
}

/// The loaded model.
pub struct Model {
    config: Config,
    indexer: Vec<i64>,
    duration: Session,
    encoder: Session,
    estimator: Session,
    vocoder: Session,
}

impl Model {
    /// Loads the model from `dir` (`onnx/` of the release), each network
    /// running on `threads` threads.
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let config: Config = serde_json::from_slice(&std::fs::read(dir.join("tts.json"))?)
            .wrap_err("reading tts.json")?;
        let indexer: Vec<i64> =
            serde_json::from_slice(&std::fs::read(dir.join("unicode_indexer.json"))?)
                .wrap_err("reading unicode_indexer.json")?;
        let session = |name: &str| -> Result<Session> {
            Session::builder()
                .map_err(|error| eyre!("{error}"))?
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
                .map_err(|error| eyre!("{error}"))?
                .with_intra_threads(threads.max(1))
                .map_err(|error| eyre!("{error}"))?
                .commit_from_file(dir.join(name))
                .map_err(|error| eyre!("loading {name}: {error}"))
        };
        Ok(Self {
            config,
            indexer,
            duration: session("duration_predictor.onnx")?,
            encoder: session("text_encoder.onnx")?,
            estimator: session("vector_estimator.onnx")?,
            vocoder: session("vocoder.onnx")?,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.ae.sample_rate
    }

    /// `sentence` in `language` (one of [`text::LANGUAGES`]) read by
    /// `style`: its audio at [`Self::sample_rate`], cut to its predicted
    /// length.
    pub fn speak(&mut self, sentence: &str, language: &str, style: &Style) -> Result<Vec<f32>> {
        self.speak_with(sentence, language, style, STEPS)
    }

    /// [`Self::speak`] with `steps` denoising steps.
    pub fn speak_with(
        &mut self,
        sentence: &str,
        language: &str,
        style: &Style,
        steps: usize,
    ) -> Result<Vec<f32>> {
        let prepared = text::prepare(sentence, language);
        let ids = text::ids(&prepared, &self.indexer);
        let length = ids.len();
        if length == 0 {
            return Ok(Vec::new());
        }
        let text_ids = tensor([1, length], ids)?;
        let text_mask = tensor([1, 1, length], vec![1.0_f32; length])?;
        let style_dp = tensor3(&style.dp)?;
        let style_ttl = tensor3(&style.ttl)?;

        let outputs = self
            .duration
            .run(ort::inputs! {
                "text_ids" => &text_ids,
                "style_dp" => &style_dp,
                "text_mask" => &text_mask,
            })
            .map_err(|error| eyre!("duration: {error}"))?;
        let (_, duration) = outputs["duration"]
            .try_extract_tensor::<f32>()
            .map_err(|error| eyre!("duration: {error}"))?;
        let seconds = duration.first().copied().unwrap_or_default() / SPEED;
        drop(outputs);

        let outputs = self
            .encoder
            .run(ort::inputs! {
                "text_ids" => &text_ids,
                "style_ttl" => &style_ttl,
                "text_mask" => &text_mask,
            })
            .map_err(|error| eyre!("text encoder: {error}"))?;
        let (shape, embedding) = outputs["text_emb"]
            .try_extract_tensor::<f32>()
            .map_err(|error| eyre!("text encoder: {error}"))?;
        let text_emb = tensor(dims3(shape)?, embedding.to_vec())?;
        drop(outputs);

        let latent = Latent::new(seconds, &self.config);
        let mut noisy = latent.noise();
        let latent_mask = tensor([1, 1, latent.frames], vec![1.0_f32; latent.frames])?;
        let total = tensor([1], vec![steps as f32])?;
        for step in 0..steps {
            let current = tensor([1], vec![step as f32])?;
            let input = tensor([1, latent.channels, latent.frames], noisy)?;
            let outputs = self
                .estimator
                .run(ort::inputs! {
                    "noisy_latent" => &input,
                    "text_emb" => &text_emb,
                    "style_ttl" => &style_ttl,
                    "latent_mask" => &latent_mask,
                    "text_mask" => &text_mask,
                    "current_step" => &current,
                    "total_step" => &total,
                })
                .map_err(|error| eyre!("vector estimator: {error}"))?;
            let (_, denoised) = outputs["denoised_latent"]
                .try_extract_tensor::<f32>()
                .map_err(|error| eyre!("vector estimator: {error}"))?;
            noisy = denoised.to_vec();
        }

        let input = tensor([1, latent.channels, latent.frames], noisy)?;
        let keep = (self.sample_rate() as f32 * seconds) as usize;
        let outputs = self
            .vocoder
            .run(ort::inputs! { "latent" => &input })
            .map_err(|error| eyre!("vocoder: {error}"))?;
        let (_, wav) = outputs["wav_tts"]
            .try_extract_tensor::<f32>()
            .map_err(|error| eyre!("vocoder: {error}"))?;
        Ok(wav[..keep.min(wav.len())].to_vec())
    }
}

/// The latent's size for a sentence of `seconds` (`sample_noisy_latent`
/// upstream, batch of one, so its mask is all ones).
struct Latent {
    channels: usize,
    frames: usize,
}

impl Latent {
    fn new(seconds: f32, config: &Config) -> Self {
        let samples = (seconds.max(0.0) * config.ae.sample_rate as f32) as usize;
        let chunk = config.ae.base_chunk_size * config.ttl.chunk_compress_factor;
        Self {
            channels: config.ttl.latent_dim * config.ttl.chunk_compress_factor,
            frames: samples.div_ceil(chunk).max(1),
        }
    }

    /// Standard normal noise to start from.
    fn noise(&self) -> Vec<f32> {
        let mut normal = Normal::seeded();
        (0..self.channels * self.frames)
            .map(|_| normal.next())
            .collect()
    }
}

/// Standard normal samples (Box–Muller over xorshift64*), seeded from the
/// system's random source.
struct Normal {
    state: u64,
    spare: Option<f32>,
}

impl Normal {
    fn seeded() -> Self {
        let mut seed = [0_u8; 8];
        if getrandom::fill(&mut seed).is_err() {
            seed = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |elapsed| elapsed.as_nanos() as u64))
            .to_le_bytes();
        }
        Self {
            state: u64::from_le_bytes(seed) | 1,
            spare: None,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let bits = self.state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
        // In (0, 1]: never 0, so the logarithm is finite.
        (bits as f64 + 1.0) / (1_u64 << 53) as f64
    }

    fn next(&mut self) -> f32 {
        if let Some(spare) = self.spare.take() {
            return spare;
        }
        let radius = (-2.0 * self.uniform().ln()).sqrt();
        let angle = std::f64::consts::TAU * self.uniform();
        self.spare = Some((radius * angle.sin()) as f32);
        (radius * angle.cos()) as f32
    }
}

fn tensor<
    T: ort::value::PrimitiveTensorElementType + Clone + std::fmt::Debug + 'static,
    const N: usize,
>(
    shape: [usize; N],
    data: Vec<T>,
) -> Result<Tensor<T>> {
    Tensor::from_array((shape, data)).map_err(|error| eyre!("{error}"))
}

fn tensor3((dims, data): &(Vec<usize>, Vec<f32>)) -> Result<Tensor<f32>> {
    tensor([dims[0], dims[1], dims[2]], data.clone())
}

fn dims3(shape: &ort::value::Shape) -> Result<[usize; 3]> {
    match shape
        .iter()
        .map(|&dim| usize::try_from(dim))
        .collect::<Vec<_>>()[..]
    {
        [Ok(a), Ok(b), Ok(c)] => Ok([a, b, c]),
        _ => Err(eyre!("unexpected text embedding shape {shape:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            ae: AutoEncoder {
                sample_rate: 44_100,
                base_chunk_size: 512,
            },
            ttl: TextToLatent {
                chunk_compress_factor: 6,
                latent_dim: 24,
            },
        }
    }

    #[test]
    fn the_latent_covers_the_predicted_length_in_whole_chunks() {
        let latent = Latent::new(1.0, &config());
        assert_eq!(latent.channels, 144);
        // 44 100 samples in chunks of 3 072.
        assert_eq!(latent.frames, 15);
        assert_eq!(Latent::new(0.0, &config()).frames, 1);
    }

    #[test]
    fn noise_is_roughly_standard_normal() {
        let mut normal = Normal::seeded();
        let samples: Vec<f32> = (0..20_000).map(|_| normal.next()).collect();
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let variance =
            samples.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / samples.len() as f32;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.08, "variance {variance}");
        assert!(samples.iter().all(|x| x.is_finite()));
    }
}
