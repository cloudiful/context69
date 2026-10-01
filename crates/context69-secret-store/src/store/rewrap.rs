//! Master-key recovery and rotation: the rewrap run.
//!
//! A rewrap re-seals the stored ciphertext of every catalogue secret the outgoing
//! master key can open, under an incoming key registered at a strictly higher key
//! version. It is the operation the recovery runbook in `docs/configuration.md`
//! drives, and it is the only path that moves an existing secret from one key to
//! another without its value ever being written anywhere else.
//!
//! It lives in its own module because it is the one store operation that needs
//! *two* deployments at once. Keeping it apart from the single-deployment
//! operations in [`super`] is what keeps those readable: nothing in `get`, `put`,
//! `rotate`, `delete`, `has`, or `get_or_create` has to know that a second cipher
//! exists.
//!
//! What the run guarantees:
//!
//! * **Both ends keyed, and the version strictly higher.** An unkeyed end or a
//!   target version that is not above the source version is refused before any
//!   row is read, so the run can never half-apply, and it can never move a
//!   deployment back onto a retired key.
//! * **Purpose-bound, and only through the catalogue.** The worklist is
//!   enumerated per [`SecretPurpose`], so a row is opened with the purpose that
//!   owns it and re-sealed under that same purpose; a value cannot move between
//!   owners.
//! * **Resumable.** The worklist is filtered on the *source* key version, so a
//!   row an earlier run already re-sealed is not offered again, and a row this
//!   run has not reached still opens with the outgoing key. Nothing is deleted,
//!   and a failure stops the run rather than skipping a row.
//! * **Counts only.** A value is passed straight from the source cipher to the
//!   target store and is never named, returned, or logged; the caller gets
//!   bounded counts and, on failure, an error that carries no value.
//!
//! Legacy plaintext rows are not touched: they hold no frame to open, so they
//! belong to the separately reviewed backfill, not to a key rotation.

use std::fmt;

use crate::{error::SecretStoreError, purpose::SecretPurpose};

use super::SecretStore;

/// Bounded counts from one rewrap run. Aggregates only: no key name, no value,
/// no per-purpose breakdown, so a report is safe to log and to paste into an
/// audit note. After a run, `rewrapped` rows must sit at the target key version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RewrapReport {
    /// Rows re-sealed under the target key version.
    pub rewrapped: usize,
    /// Catalogue purposes that contributed at least one of those rows.
    pub purposes: usize,
}

/// Why a rewrap run did not complete. No variant carries a key name, a value, or
/// a stored row, so the enum is safe to log; every refusal is decided from the
/// two deployments before a row is read, so a rejected run cannot half-apply.
#[derive(Debug)]
pub enum RewrapError {
    /// The source deployment has no master key, so no sealed row can be opened.
    SourceMasterKeyMissing,
    /// The target deployment has no master key to seal with.
    TargetMasterKeyMissing,
    /// The target key version must be strictly above the source version, so a
    /// rewrap can never move a deployment onto a retired key or rewrite a row
    /// with the key that already sealed it.
    TargetKeyVersionNotNewer {
        /// Version the source key is registered under.
        source: u32,
        /// Version the target key was offered under.
        target: u32,
    },
    /// A row could not be opened or re-sealed.
    Store(SecretStoreError),
}

impl fmt::Display for RewrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceMasterKeyMissing => formatter.write_str(
                "rewrap needs the source deployment's master key; set secret_store.master_key",
            ),
            Self::TargetMasterKeyMissing => formatter
                .write_str("rewrap needs a target master key from the deployment environment"),
            Self::TargetKeyVersionNotNewer { source, target } => write!(
                formatter,
                "rewrap target key version {target} must be greater than the source key version {source}"
            ),
            Self::Store(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for RewrapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SecretStoreError> for RewrapError {
    fn from(error: SecretStoreError) -> Self {
        Self::Store(error)
    }
}

impl SecretStore {
    /// Re-seals every catalogue secret the source key can open into the target
    /// key: the master-key recovery and rotation operation. `self` is the target
    /// that re-seals, `source` is the outgoing deployment that opens the rows.
    ///
    /// # Errors
    ///
    /// [`RewrapError`]: a refusal for an unkeyed end or a non-increasing target
    /// version, or the first row that could not be opened or re-sealed.
    pub async fn rewrap_secrets_from(
        &self,
        source: &SecretStore,
    ) -> Result<RewrapReport, RewrapError> {
        let source_cipher = source
            .cipher
            .as_ref()
            .ok_or(RewrapError::SourceMasterKeyMissing)?;
        let target_cipher = self
            .cipher
            .as_ref()
            .ok_or(RewrapError::TargetMasterKeyMissing)?;
        if target_cipher.key_version() <= source_cipher.key_version() {
            return Err(RewrapError::TargetKeyVersionNotNewer {
                source: source_cipher.key_version(),
                target: target_cipher.key_version(),
            });
        }
        // Same bound and same refusal as `seal_for_storage`: a version the
        // table's `integer` column cannot hold is a configuration failure.
        let source_version = i32::try_from(source_cipher.key_version())
            .map_err(|_| RewrapError::Store(SecretStoreError::MasterKeyNotConfigured))?;

        let mut report = RewrapReport::default();
        for purpose in SecretPurpose::ALL {
            let keys = source
                .db
                .list_sealed_secret_keys(purpose.as_str(), source_version)
                .await
                .map_err(SecretStoreError::from)?;
            if keys.is_empty() {
                continue;
            }
            report.purposes += 1;
            for key_name in keys {
                // Straight from the source cipher into the target store under the
                // same purpose. A key that vanished since the enumeration stops
                // the run rather than being silently skipped.
                let opened = source
                    .get(purpose, &key_name)
                    .await?
                    .ok_or(RewrapError::Store(SecretStoreError::SecretNotFound))?;
                self.rotate(purpose, &key_name, opened.expose()).await?;
                report.rewrapped += 1;
            }
        }
        Ok(report)
    }
}
