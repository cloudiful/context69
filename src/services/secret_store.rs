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

/// Builds a store handle from deployment configuration.
///
/// Several handles onto one configuration are expected — the application, the
/// services that need one at construction time, a background task — so this
/// neither warns nor allocates anything the caller cannot see. Reporting the
/// unconfigured state once per process is
/// [`crate::services::app::config_hydration`]'s job.
///
/// The master key is a deployment input, so it is read from configuration here
/// and handed to the cipher; it is never written to PostgreSQL, a log line, an
/// error, or a response.
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

/// A store handle with no master key, for a caller that has no deployment
/// configuration to hand.
///
/// Secrets then round-trip in the legacy plaintext representation and are read
/// back from their legacy column, which is exactly the behaviour of a
/// deployment that has not configured a key. It is the same code path, not a
/// bypass, so a caller that only has a pool still goes through the store.
pub fn build_unkeyed(db: &Database) -> SecretStore {
    build(db, &SecretStoreConfig::default())
        .expect("a store with no master key cannot fail to build")
}
