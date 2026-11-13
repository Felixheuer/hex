//! One-time migration, remove after 2026-12: what Agent-Desk was called
//! before, Hex (`anomaly.hex`). Only the one-time migrations that keep the
//! person's data read these names: the data and config folders
//! (`app_paths::take_over`), the Keychain items (`assistant::secrets`), and
//! the move from `/Applications/Hex.app` (`updater::rename`). Nothing new
//! is ever written under them, and nothing else answers to them.
//!
//! The product rename leaves this one file alone.

use super::Variant;

/// One variant's names under the old identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Identity {
    /// The app's name (`Hex Dev`), as `Hex Dev.app` was called.
    pub name: &'static str,
    pub bundle_id: &'static str,
    /// Its folder under Application Support and `~/.config`.
    pub folder: &'static str,
}

pub const fn identity(variant: Variant) -> Identity {
    match variant {
        Variant::Live => Identity {
            name: "Hex",
            bundle_id: "anomaly.hex",
            folder: "hex",
        },
        Variant::Dev => Identity {
            name: "Hex Dev",
            bundle_id: "anomaly.hex.dev",
            folder: "hex-dev",
        },
        Variant::Test => Identity {
            name: "Hex Test",
            bundle_id: "anomaly.hex.test",
            folder: "hex-test",
        },
    }
}

/// The service the assistant's API keys were saved under (Keychain, the
/// Secret Service, Credential Manager).
pub const KEYCHAIN_SERVICE: &str = "Hex Assistant";

/// Beside the installed bundle, where an old Hex unpacked an update and
/// left the bundle it replaced (`.Hex-update-<version>`).
pub const UPDATE_STAGING_PREFIX: &str = ".Hex-update-";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_had_its_own_old_names() {
        for variant in [Variant::Live, Variant::Dev, Variant::Test] {
            let old = identity(variant);
            assert_ne!(old.folder, variant.folder());
            assert_ne!(old.bundle_id, variant.bundle_id());
            assert_ne!(old.name, variant.name());
        }
        assert_eq!(identity(Variant::Live).folder, "hex");
        assert_eq!(identity(Variant::Dev).bundle_id, "anomaly.hex.dev");
    }
}
