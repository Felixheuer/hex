//! Supertonic 3's files, pinned by size and SHA-256, and their download into
//! the models folder (`supertonic-3/`), each through `net` (the computer's
//! proxy and trusted roots) to a `.partial` file that is checked before it
//! takes the real name.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use color_eyre::eyre::{Result, bail, eyre};
use sha2::{Digest, Sha256};

/// Where the files come from: Supertone's archived release, at the commit
/// these pins were taken from. A mirror only needs the same paths.
pub const SOURCE: &str = "https://huggingface.co/supertone-oss-archive/supertonic-3/resolve/aafc6e32416a594460b32413efc49d7fe4ce6d46";

/// The folder under the models folder that holds them.
pub const FOLDER: &str = "supertonic-3";

/// One pinned file, by its path in the release.
pub struct File {
    pub path: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// Every file the voice needs. The model license (Open RAIL-M) comes along
/// and is kept beside the weights.
pub const FILES: &[File] = &[
    file(
        "LICENSE",
        15_007,
        "0d944a9110fed9a9602d60e0423a272903e7bd21ab060490774efc77c2275e9f",
    ),
    file(
        "onnx/tts.json",
        8_253,
        "42078d3aef1cd43ab43021f3c54f47d2d75ceb4e75f627f118890128b06a0d09",
    ),
    file(
        "onnx/unicode_indexer.json",
        277_676,
        "9bf7346e43883a81f8645c81224f786d43c5b57f3641f6e7671a7d6c493cb24f",
    ),
    file(
        "onnx/duration_predictor.onnx",
        3_700_147,
        "c3eb91414d5ff8a7a239b7fe9e34e7e2bf8a8140d8375ffb14718b1c639325db",
    ),
    file(
        "onnx/text_encoder.onnx",
        36_416_150,
        "c7befd5ea8c3119769e8a6c1486c4edc6a3bc8365c67621c881bbb774b9902ff",
    ),
    file(
        "onnx/vector_estimator.onnx",
        256_534_781,
        "883ac868ea0275ef0e991524dc64f16b3c0376efd7c320af6b53f5b780d7c61c",
    ),
    file(
        "onnx/vocoder.onnx",
        101_424_195,
        "085de76dd8e8d5836d6ca66826601f615939218f90e519f70ee8a36ed2a4c4ba",
    ),
    file(
        "voice_styles/F1.json",
        292_046,
        "bbdec6ee00231c2c742ad05483df5334cab3b52fda3ba38e6a07059c4563dbc2",
    ),
    file(
        "voice_styles/F2.json",
        292_423,
        "7c722c6a72707b1a77f035d67f0d1351ba187738e06f7683e8c72b1df3477fc6",
    ),
    file(
        "voice_styles/F3.json",
        290_794,
        "12f6ef2573baa2defa1128069cb59f203e3ab67c92af77b42df8a0e3a2f7c6ab",
    ),
    file(
        "voice_styles/F4.json",
        291_808,
        "c2fa764c1225a76dfc3e2c73e8aa4f70d9ee48793860eb34c295fff01c2e032b",
    ),
    file(
        "voice_styles/F5.json",
        291_479,
        "45966e73316415626cf41a7d1c6f3b4c70dbc1ba2bee5c1978ef0ce33244fc8d",
    ),
    file(
        "voice_styles/M1.json",
        291_748,
        "e35604687f5d23694b8e91593a93eec0e4eca6c0b02bb8ed69139ab2ea6b0a5b",
    ),
    file(
        "voice_styles/M2.json",
        292_055,
        "b76cbf62bac707c710cf0ae5aba5e31eea1a6339a9734bfae33ab98499534a50",
    ),
    file(
        "voice_styles/M3.json",
        290_198,
        "ea1ac35ccb91b0d7ecad533a2fbd0eec10c91513d8951e3b25fbba99954e159b",
    ),
    file(
        "voice_styles/M4.json",
        291_522,
        "ca8eefad4fcd989c9379032ff3e50738adc547eeb5e221b82593a6d7b3bac303",
    ),
    file(
        "voice_styles/M5.json",
        291_469,
        "dd22b92740314321f8ae11c5e87f8dd60d060f15dd3a632b5adf77f471f77af2",
    ),
];

const fn file(path: &'static str, bytes: u64, sha256: &'static str) -> File {
    File {
        path,
        bytes,
        sha256,
    }
}

/// How much there is to download, in bytes.
pub fn total_bytes() -> u64 {
    FILES.iter().map(|file| file.bytes).sum()
}

/// The folder the files are in, installed or not.
pub fn folder() -> Result<PathBuf> {
    Ok(crate::app_paths::models_dir()?.join(FOLDER))
}

/// Whether every file is there at its pinned size (each was checked by
/// its SHA-256 as it arrived).
pub fn installed() -> bool {
    folder().is_ok_and(|folder| installed_in(&folder))
}

pub fn installed_in(folder: &Path) -> bool {
    FILES.iter().all(|file| {
        fs::metadata(folder.join(file.path)).is_ok_and(|metadata| metadata.len() == file.bytes)
    })
}

/// Downloads what's missing into the models folder; `done` hears the bytes
/// in place so far. Stops with an error when `canceled` is set.
pub fn download(canceled: &AtomicBool, done: &AtomicU64) -> Result<PathBuf> {
    let folder = folder()?;
    download_into(&folder, SOURCE, canceled, done)?;
    Ok(folder)
}

pub fn download_into(
    folder: &Path,
    source: &str,
    canceled: &AtomicBool,
    done: &AtomicU64,
) -> Result<()> {
    let mut finished = 0;
    for file in FILES {
        if canceled.load(Ordering::Relaxed) {
            bail!("Canceled");
        }
        let destination = folder.join(file.path);
        if fs::metadata(&destination).is_ok_and(|metadata| metadata.len() == file.bytes) {
            finished += file.bytes;
            done.store(finished, Ordering::Relaxed);
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let partial = destination.with_extension("partial");
        if fs::metadata(&partial).is_ok_and(|metadata| metadata.len() > file.bytes) {
            fs::remove_file(&partial)?;
        }
        let url = format!("{source}/{}", file.path);
        let before = finished;
        crate::net::download(
            &crate::net::Download {
                url: &url,
                user_agent: &format!("hex/{}", crate::VERSION),
                size: Some(file.bytes),
                canceled: Some(canceled),
            },
            &partial,
            |bytes| done.store(before + bytes, Ordering::Relaxed),
        )
        .map_err(|error| eyre!("{error}"))?;
        if let Err(error) = verify(&partial, file) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        fs::rename(&partial, &destination)?;
        finished += file.bytes;
        done.store(finished, Ordering::Relaxed);
    }
    Ok(())
}

/// Whether `path` is `file`: its size, then its SHA-256.
fn verify(path: &Path, file: &File) -> Result<()> {
    let length = fs::metadata(path)?.len();
    if length != file.bytes {
        bail!("{} is {length} bytes, not {}", file.path, file.bytes);
    }
    let digest = sha256_of(path)?;
    if digest != file.sha256 {
        bail!("{} didn't match its checksum", file.path);
    }
    Ok(())
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut reader = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_is_pinned_once_with_a_full_checksum() {
        let mut paths: Vec<_> = FILES.iter().map(|file| file.path).collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), FILES.len());
        for file in FILES {
            assert_eq!(file.sha256.len(), 64, "{}", file.path);
            assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(file.bytes > 0);
        }
        assert!(FILES.iter().any(|file| file.path == "LICENSE"));
        // About 400 MB: the fp32 release.
        assert!((395_000_000..405_000_000).contains(&total_bytes()));
    }

    #[test]
    fn the_source_is_pinned_to_a_commit() {
        let revision = SOURCE.rsplit('/').next().unwrap();
        assert_eq!(revision.len(), 40);
        assert!(SOURCE.starts_with("https://"));
    }

    #[test]
    fn a_file_is_checked_by_size_and_checksum() {
        let dir = std::env::temp_dir().join(format!("hex-neural-verify-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x");
        fs::write(&path, b"abc").unwrap();
        let good = File {
            path: "x",
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(verify(&path, &good).is_ok());
        let wrong_size = File { bytes: 4, ..good };
        assert!(verify(&path, &wrong_size).is_err());
        let wrong_sum = File {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..wrong_size
        };
        let wrong_sum = File {
            bytes: 3,
            ..wrong_sum
        };
        assert!(verify(&path, &wrong_sum).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
