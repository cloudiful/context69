//! The shared encrypted secret store.
//!
//! One application-owned store for every persisted reversible runtime
//! credential, usable by the root service and by the translation/extraction
//! crates without a dependency cycle. The crate is a leaf: it depends only on
//! the [`context69_secret_crypto`] primitive, SQLx, and logging, so a dependent
//! service can reach the store without depending on the application.
//!
//! Three layers, each with one job:
//!
//! * [`purpose`] — the catalogue. One [`SecretPurpose`] per owning category and
//!   a bounded [`SecretKeyName`] for each secret within it. Adding a category
//!   means adding a variant, so an unowned purpose cannot be invented at a call
//!   site.
//! * [`database`] — the SQL boundary. One file-backed statement per operation,
//!   and the only place that can move a secret's bytes.
//! * [`store`] — the typed accessor. Purpose binding and explicit lifecycle
//!   operations for one deployment, plus the transition read and, in its
//!   `rewrap` child module, the one operation that spans two: re-sealing this
//!   store's rows under an incoming master key.
//!
//! ```no_run
//! # use context69_secret_store::{SecretDatabase, SecretPurpose, SecretStore, key_names};
//! # async fn demo(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
//! let store = SecretStore::new(SecretDatabase::new(pool), None, 1)?;
//! // Presence is metadata-only: it answers without a master key and decrypts
//! // nothing, so a `has_*` projection stays truthful on a read-only deployment.
//! let present = store
//!     .has(
//!         SecretPurpose::BrowserSessionSigningKey,
//!         key_names::BROWSER_SESSION_SIGNING_KEY,
//!     )
//!     .await?;
//! # let _ = present;
//! # Ok(())
//! # }
//! ```
//!
//! A deployment that has not configured a master key can still answer presence
//! and still read a legacy plaintext row; a sealed row it cannot open is an
//! explicit [`SecretStoreError::MasterKeyNotConfigured`], never a fallback.

mod database;
mod error;
mod purpose;
mod rows;
mod store;

pub use context69_secret_crypto as crypto;

pub use database::{SecretDatabase, StoredWrite};
pub use error::SecretStoreError;
pub use purpose::{LONGEST_KEY_NAME, LONGEST_PURPOSE, SecretKeyName, SecretPurpose, key_names};
pub use rows::{
    LEGACY_PLAINTEXT_VERSION, SEALED_CIPHERTEXT_VERSION, StoredSecret, StoredSecretBytes,
    StoredSecretMetadata, open_from_storage, seal_for_storage,
};
pub use store::{PutOutcome, RewrapError, RewrapReport, SecretStore, master_key_from_config};
