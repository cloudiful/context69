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

use crate::{config::Config, db::Database};

/// Builds a store handle from deployment configuration.
///
/// The master secret is an application-scoped input, so it is read from
/// [`Config::app`] and the ciphertext version from the store's own section; both
/// are resolved here, once, and the secret never appears at a call site. It is
/// never written to PostgreSQL, a log line, an error, or a response.
///
/// Several handles onto one configuration are expected — the application, the
/// services that need one at construction time, a background task — so this
/// neither warns nor allocates anything the caller cannot see. Reporting the
/// unconfigured state once per process is
/// [`crate::services::app::config_hydration`]'s job.
///
/// # Errors
///
/// [`SecretStoreError::MasterKeyRejected`] when a configured master secret cannot
/// be used. A blank or absent secret is the transition state, not an error.
pub fn build(db: &Database, config: &Config) -> Result<SecretStore, SecretStoreError> {
    SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        config.app.master_secret.as_deref(),
        config.secret_store.key_version,
    )
}

/// A store handle with no master secret, for a caller that has no deployment
/// configuration to hand.
///
/// Secrets then round-trip in the legacy plaintext representation and are read
/// back from their legacy column, which is exactly the behaviour of a
/// deployment that has not configured a secret. It is the same code path, not a
/// bypass, so a caller that only has a pool still goes through the store. With no
/// secret there is nothing to version, so it uses the documented default.
pub fn build_unkeyed(db: &Database) -> SecretStore {
    SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        None,
        crate::config::DEFAULT_SECRET_STORE_KEY_VERSION,
    )
    .expect("a store with no master secret cannot fail to build")
}
