//! The SQL boundary of the secret store.
//!
//! This module owns every statement the store issues and the row shapes they
//! project, and nothing else: it holds a pool, runs one file-backed query per
//! operation, and returns either a row shape or a `sqlx::Error`. Interpreting a
//! row — deciding whether it is legacy, who owns it, whether it may be opened —
//! belongs to the store above, and error mapping belongs to the caller of the
//! store.
//!
//! The statements live beside this module under `src/sql/internal_secrets/` and
//! are referenced with `query_file!` / `query_file_as!`, so each one is
//! type-checked against the live schema at compile time.

use sqlx::PgPool;

use crate::rows::{StoredSecret, StoredSecretBytes, StoredSecretMetadata};

/// The pool the secret store reads and writes through.
///
/// This is a separate handle from the application's own database type on
/// purpose: the store must stay usable by a crate that has no dependency on the
/// application, and it must not be able to reach any table it does not own.
#[derive(Clone)]
pub struct SecretDatabase {
    pool: PgPool,
}

impl std::fmt::Debug for SecretDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretDatabase")
            .field("pool", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl SecretDatabase {
    /// Wraps an existing pool. No migration, connection or write happens here.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The pool, for a caller that needs to compose a transaction around the
    /// store.
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Reads one secret row, or `None` when the key is not stored.
    ///
    /// This is the only statement that selects `value`, so it is the only path
    /// that can move a secret's bytes.
    pub async fn get_secret(&self, key_name: &str) -> Result<Option<StoredSecret>, sqlx::Error> {
        let row = sqlx::query_file_as!(
            StoredSecret,
            "src/sql/internal_secrets/get_internal_secret.sql",
            key_name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Reads one secret row's metadata, without its value.
    ///
    /// Backs the presence check: a deployment that has no master key still
    /// needs `has_*` to be truthful, and that must not require decrypting
    /// anything.
    pub async fn get_secret_metadata(
        &self,
        key_name: &str,
    ) -> Result<Option<StoredSecretMetadata>, sqlx::Error> {
        let row = sqlx::query_file_as!(
            StoredSecretMetadata,
            "src/sql/internal_secrets/has_internal_secret.sql",
            key_name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Creates a secret row and reports whether this call created it.
    ///
    /// `false` means the key was already taken and nothing was written, so the
    /// caller can read the existing value instead of overwriting it.
    pub async fn put_secret(
        &self,
        key_name: &str,
        stored: &StoredWrite<'_>,
    ) -> Result<bool, sqlx::Error> {
        let key = sqlx::query_file_scalar!(
            "src/sql/internal_secrets/put_internal_secret.sql",
            key_name,
            stored.value,
            stored.purpose,
            stored.key_version,
            stored.ciphertext_version
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(key.is_some())
    }

    /// Replaces the stored representation of an existing secret.
    ///
    /// `false` means no row carried that key, so nothing was written.
    pub async fn rotate_secret(
        &self,
        key_name: &str,
        stored: &StoredWrite<'_>,
    ) -> Result<bool, sqlx::Error> {
        let key = sqlx::query_file_scalar!(
            "src/sql/internal_secrets/rotate_internal_secret.sql",
            key_name,
            stored.value,
            stored.purpose,
            stored.key_version,
            stored.ciphertext_version
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(key.is_some())
    }

    /// Deletes a secret row and reports whether this call deleted it.
    pub async fn delete_secret(&self, key_name: &str) -> Result<bool, sqlx::Error> {
        let key = sqlx::query_file_scalar!(
            "src/sql/internal_secrets/delete_internal_secret.sql",
            key_name
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(key.is_some())
    }

    /// Lists the keys still stored in the legacy plaintext representation.
    ///
    /// Only key names come back: the transition worklist is produced without
    /// reading a single value.
    pub async fn list_unversioned_secret_keys(&self) -> Result<Vec<String>, sqlx::Error> {
        let keys = sqlx::query_file_scalar!(
            "src/sql/internal_secrets/list_unversioned_internal_secrets.sql"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(keys)
    }
}

/// Borrowed form of the columns a write needs, so a caller can hand a
/// [`crate::rows::StoredSecretBytes`] straight to a write without cloning the
/// sealed bytes.
pub struct StoredWrite<'a> {
    /// The stored representation bytes.
    pub value: &'a [u8],
    /// The owning purpose, absent for a legacy plaintext write.
    pub purpose: Option<&'a str>,
    /// Master-key version.
    pub key_version: i32,
    /// Representation marker.
    pub ciphertext_version: i32,
}

impl<'a> From<&'a StoredSecretBytes> for StoredWrite<'a> {
    fn from(bytes: &'a StoredSecretBytes) -> Self {
        Self {
            value: &bytes.value,
            purpose: bytes.purpose.as_deref(),
            key_version: bytes.key_version,
            ciphertext_version: bytes.ciphertext_version,
        }
    }
}
