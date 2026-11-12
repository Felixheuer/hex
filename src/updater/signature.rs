//! Pilot's release signature, checked on every OS. Each published file the
//! updater runs (the Linux and Windows executables, the Windows `hex.com`,
//! the macOS update ZIP) has a 64-byte ed25519 signature by Pilot's release
//! key over its SHA-256, published beside it as the `<target>-sig` target
//! (`scripts/pilot-publish.sh` signs with the private half, which lives only
//! at `~/.config/pilot-release/signing-key` on the release server). The
//! updater checks it before the file itself is fetched, so whoever controls
//! the manifest cannot name another file. macOS and Windows additionally
//! check their OS code signature when the build names a signer
//! (`HEX_UPDATE_TEAM_ID`, `HEX_AUTHENTICODE_SIGNER`).

use color_eyre::eyre::{Result, WrapErr};

/// Pilot's ed25519 release key, `.github/release/linux-signing.pub`.
pub const SIGNING_KEY: [u8; 32] = [
    0x7a, 0xcd, 0xcc, 0x24, 0xcd, 0xd8, 0xe2, 0x78, 0x9b, 0xa7, 0x5e, 0xb0, 0xff, 0xce, 0x48, 0x0c,
    0x91, 0x99, 0xb5, 0x14, 0xa2, 0xdd, 0x3f, 0xd9, 0x6d, 0xcb, 0x2d, 0x29, 0x93, 0xf5, 0xa1, 0x00,
];

/// The manifest target holding the signature of `target`.
pub fn signature_target(target: &str) -> String {
    format!("{target}-sig")
}

/// What the key signs: a fixed prefix, then the file's SHA-256 in hex.
pub fn signed_message(sha256: &str) -> String {
    format!("pilot.release-artifact.v1\n{sha256}\n")
}

/// Whether `signature` is Pilot's release key signing the file whose
/// SHA-256 is `sha256`. The manifest names that SHA-256, and the download is
/// checked against it, so this ties the bytes to the key.
pub fn verify_signature(sha256: &str, signature: &[u8]) -> Result<()> {
    verify_with(&SIGNING_KEY, sha256, signature)
}

fn verify_with(key: &[u8; 32], sha256: &str, signature: &[u8]) -> Result<()> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let signature =
        Signature::from_slice(signature).wrap_err("the update's signature is malformed")?;
    VerifyingKey::from_bytes(key)
        .wrap_err("invalid release key")?
        .verify_strict(signed_message(sha256).as_bytes(), &signature)
        .wrap_err("the update is not signed by Pilot's release key")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn signatures_bind_the_release_key_to_the_sha256() {
        use ed25519_dalek::{Signer, SigningKey};
        let key = SigningKey::from_bytes(&[7; 32]);
        let public = key.verifying_key().to_bytes();
        let sha = "a".repeat(64);
        let signature = key.sign(signed_message(&sha).as_bytes()).to_bytes();
        assert!(verify_with(&public, &sha, &signature).is_ok());
        assert!(verify_with(&public, &"b".repeat(64), &signature).is_err());
        assert!(verify_with(&SIGNING_KEY, &sha, &signature).is_err());
        assert!(verify_with(&public, &sha, &signature[..63]).is_err());
    }

    /// The key's SubjectPublicKeyInfo in base64, as the PEM files and the
    /// installers carry it.
    fn encoded_key() -> String {
        // SubjectPublicKeyInfo for Ed25519: a fixed 12-byte prefix, then the key.
        let mut der = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        der.extend_from_slice(&SIGNING_KEY);
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in der.chunks(3) {
            let bits = chunk.iter().enumerate().fold(0u32, |bits, (index, byte)| {
                bits | u32::from(*byte) << (16 - 8 * index)
            });
            for index in 0..4 {
                encoded.push(if index <= chunk.len() {
                    char::from(ALPHABET[(bits >> (18 - 6 * index) & 63) as usize])
                } else {
                    '='
                });
            }
        }
        encoded
    }

    #[test]
    fn the_compiled_key_is_the_published_one() {
        // .github/release/ is outside the Nix source; skip there.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let Ok(pem) = fs::read_to_string(root.join(".github/release/linux-signing.pub")) else {
            return;
        };
        let encoded = encoded_key();
        assert!(pem.lines().any(|line| line == encoded), "{encoded}");
        let installer = fs::read_to_string(root.join(".github/release/install-linux.sh")).unwrap();
        assert!(installer.contains(&format!("\nsigning_key={encoded}\n")));
        assert!(installer.contains(r"pilot.release-artifact.v1\n%s\n"));
        let publish = fs::read_to_string(root.join("scripts/pilot-publish.sh")).unwrap();
        assert!(publish.contains(r"pilot.release-artifact.v1\n%s\n"));
    }

    /// Checks a published release with this build's key and rules:
    ///
    /// ```sh
    /// PILOT_CHECK_MANIFEST=stable.json PILOT_CHECK_SIGNATURE=hex.sig \
    ///   PILOT_CHECK_TARGET=linux-x64 cargo test --bin hex published_release -- --ignored
    /// ```
    ///
    /// `PILOT_CHECK_FILE`, when set, must be the target itself.
    #[test]
    #[ignore = "checks a downloaded release; set PILOT_CHECK_MANIFEST"]
    fn published_release_verifies() {
        let manifest = fs::read_to_string(std::env::var("PILOT_CHECK_MANIFEST").unwrap()).unwrap();
        let target_name = std::env::var("PILOT_CHECK_TARGET").unwrap();
        let channel = std::env::var("PILOT_CHECK_CHANNEL").unwrap_or_else(|_| "stable".into());
        let parsed = super::super::protocol::parse_manifest(&manifest).expect("a valid manifest");
        let install = super::super::protocol::Install {
            version: "0.0.0".into(),
            target: target_name.clone(),
            distribution: "direct".into(),
            rollout_bucket: 0,
            sha256: None,
            channel: Some(channel.clone()),
        };
        let decision = super::super::protocol::decide(
            super::super::APP,
            super::super::ARTIFACT,
            &channel,
            &manifest,
            &install,
        );
        assert!(
            matches!(decision, super::super::protocol::Decision::Install { .. }),
            "{decision:?}"
        );
        let target = parsed.release.target(&target_name).expect("the target");
        let signature_entry = parsed
            .release
            .target(&signature_target(&target_name))
            .expect("the signature target");
        let signature = fs::read(std::env::var("PILOT_CHECK_SIGNATURE").unwrap()).unwrap();
        assert_eq!(signature.len() as u64, signature_entry.size);
        assert_eq!(
            crate::transcription::models::sha256_hex(
                Path::new(&std::env::var("PILOT_CHECK_SIGNATURE").unwrap()),
                None
            )
            .unwrap(),
            signature_entry.sha256
        );
        verify_signature(&target.sha256, &signature).unwrap();
        if let Ok(file) = std::env::var("PILOT_CHECK_FILE") {
            let path = Path::new(&file);
            assert_eq!(fs::metadata(path).unwrap().len(), target.size);
            assert_eq!(
                crate::transcription::models::sha256_hex(path, None).unwrap(),
                target.sha256
            );
        }
        println!(
            "Pilot {} {target_name}: manifest valid, signature verifies with the compiled key",
            parsed.release.version
        );
    }
}
