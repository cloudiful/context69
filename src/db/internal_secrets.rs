use anyhow::Result;

use super::Database;

/// One secret row exactly as stored.
///
/// `value` is opaque here: it is a versioned frame when
/// [`Self::ciphertext_version`] is nonzero and a legacy plaintext value when it
/// is zero. Only the store above this layer decides which, and it is the only
/// place a value is turned back into plaintext.
#[derive(Clone)]
pub struct StoredInternalSecret {
    /// The stored bytes, framed or legacy.
    pub value: Vec<u8>,
    /// Owning purpose, absent on a legacy row.
    pub purpose: Option<String>,
    /// Master-key version the value was sealed with.
    pub key_version: i32,
    /// Representation marker: zero is legacy plaintext.
    pub ciphertext_version: i32,
}

impl StoredInternalSecret {
    /// Whether these bytes are the legacy plaintext representation.
    pub fn is_legacy_plaintext(&self) -> bool {
        self.ciphertext_version == 0
    }
}

/// Reports the row's metadata and byte count, never its bytes, so a stored row
/// can be described in a log or an audit note.
impl std::fmt::Debug for StoredInternalSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredInternalSecret")
            .field("purpose", &self.purpose)
            .field("key_version", &self.key_version)
            .field("ciphertext_version", &self.ciphertext_version)
            .field("value_len", &self.value.len())
            .finish()
    }
}

impl Database {
    /// Reads one secret row, or `None` when the key is not stored.
    pub async fn get_internal_secret(&self, key: &str) -> Result<Option<StoredInternalSecret>> {
        let row = sqlx::query_file_as!(
            StoredInternalSecret,
            "src/sql/db/internal_secrets/get_internal_secret.sql",
            key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Creates a secret row and reports whether this call created it.
    ///
    /// `false` means the key was already taken and nothing was written, so the
    /// caller can read the existing value instead of overwriting it.
    pub async fn put_internal_secret(
        &self,
        key: &str,
        value: &[u8],
        purpose: Option<&str>,
        key_version: i32,
        ciphertext_version: i32,
    ) -> Result<bool> {
        let key = sqlx::query_file_scalar!(
            "src/sql/db/internal_secrets/put_internal_secret.sql",
            key,
            value,
            purpose,
            key_version,
            ciphertext_version
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(key.is_some())
    }

    /// Replaces the stored representation of an existing secret.
    ///
    /// `false` means no row carried that key, so nothing was written.
    pub async fn rotate_internal_secret(
        &self,
        key: &str,
        value: &[u8],
        purpose: Option<&str>,
        key_version: i32,
        ciphertext_version: i32,
    ) -> Result<bool> {
        let key = sqlx::query_file_scalar!(
            "src/sql/db/internal_secrets/rotate_internal_secret.sql",
            key,
            value,
            purpose,
            key_version,
            ciphertext_version
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(key.is_some())
    }

    /// Deletes a secret row and reports whether this call deleted it.
    pub async fn delete_internal_secret(&self, key: &str) -> Result<bool> {
        let key = sqlx::query_file_scalar!(
            "src/sql/db/internal_secrets/delete_internal_secret.sql",
            key
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(key.is_some())
    }

    /// Lists the keys still stored in the legacy plaintext representation.
    ///
    /// Only key names come back: the transition worklist is produced without
    /// reading a single value.
    pub async fn list_unversioned_internal_secrets(&self) -> Result<Vec<String>> {
        let keys = sqlx::query_file_scalar!(
            "src/sql/db/internal_secrets/list_unversioned_internal_secrets.sql"
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(keys)
    }
}
