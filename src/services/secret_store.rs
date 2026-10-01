//! The application's view of the shared encrypted secret store.
//!
//! The store itself — the purpose catalogue, the SQL boundary, and the typed
//! accessor — lives in the leaf [`context69_secret_store`] crate, so the
//! translation and extraction crates can reach the same implementation without a
//! dependency cycle and there is only ever one representation of a stored
//! secret. This module is the application's stable name for that seam, so a
//! service depends on `crate::services::secret_store` and never on the leaf's
//! module layout.
//!
//! Nothing here re-implements anything. The re-exports below are the whole
//! contract the application consumes.

pub use context69_secret_store::{
    LEGACY_PLAINTEXT_VERSION, SEALED_CIPHERTEXT_VERSION, SecretKeyName, SecretPurpose, SecretStore,
    SecretStoreError, key_names,
};

use crate::{config::SecretStoreConfig, db::Database};

/// Builds the application's store over the existing pool.
///
/// The master key is a deployment input, so it is read from configuration here
/// and handed to the cipher; it is never written to PostgreSQL, a log line, an
/// error, or a response. The leaf's own `warn!` reports the unconfigured state
/// once, at construction.
///
/// # Errors
///
/// [`SecretStoreError::MasterKeyRejected`] when a configured master key cannot be
/// used. A blank or absent key is the transition state, not an error.
pub fn build(db: &Database, config: &SecretStoreConfig) -> Result<SecretStore, SecretStoreError> {
    SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        config.master_key.as_deref(),
        config.key_version,
    )
}
