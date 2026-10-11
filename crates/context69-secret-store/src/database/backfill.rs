//! The startup sealing backfill's SQL boundary.
//!
//! Three statements, all in one transaction each, and nothing else: claim one
//! bounded batch of legacy rows and hold them, convert one claimed row under the
//! exact legacy marker tuple, and count what is left. The orchestration that
//! sequences them — the transaction, the loop, the decision to stop — belongs
//! to [`crate::store`]; what a statement selects and how it guards itself lives
//! here.
//!
//! It is a child module of [`super`] because that module's contract is *this
//! module owns every statement the store issues*: keeping these three beside it
//! rather than inside it is what lets `database.rs` stay a list of operations
//! instead of growing a transaction protocol.

use sqlx::{Postgres, Transaction};

use super::SecretDatabase;
use crate::database::StoredWrite;

/// One batch's worth of rows. Fixed rather than configurable: it bounds how many
/// secrets one transaction holds open and one failure rolls back.
pub(crate) const LEGACY_BATCH_LIMIT: i64 = 100;

/// One claimed legacy row, as the claim statement projects it.
///
/// The value is carried because the caller has to seal it while the row is
/// locked; `Debug` therefore reports the byte count and never the bytes, like
/// every other row shape in this crate.
pub(crate) struct LegacySecretRow {
    /// The row key, which is also what the guarded update matches on.
    pub key: String,
    /// The stored bytes, still plaintext.
    pub value: Vec<u8>,
    /// Owning purpose, always absent while the row is legacy.
    pub purpose: Option<String>,
    /// Master-key version, zero for a legacy row.
    pub key_version: i32,
    /// Representation marker, zero for a legacy row.
    pub ciphertext_version: i32,
}

impl std::fmt::Debug for LegacySecretRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacySecretRow")
            .field("purpose", &self.purpose)
            .field("key_version", &self.key_version)
            .field("ciphertext_version", &self.ciphertext_version)
            .field("value_len", &self.value.len())
            .finish()
    }
}

impl SecretDatabase {
    /// Opens the transaction one sealing batch runs in.
    pub(crate) async fn begin_sealing_batch(
        &self,
    ) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
        self.pool().begin().await
    }

    /// Claims up to `limit` legacy rows, ordered and locked, for conversion.
    ///
    /// The rows stay locked until the caller commits or rolls back, so sealing
    /// happens against exactly the bytes that were claimed.
    pub(crate) async fn claim_legacy_secrets(
        transaction: &mut Transaction<'_, Postgres>,
        limit: i64,
    ) -> Result<Vec<LegacySecretRow>, sqlx::Error> {
        sqlx::query_file_as!(
            LegacySecretRow,
            "src/sql/internal_secrets/claim_legacy_internal_secrets.sql",
            limit
        )
        .fetch_all(&mut **transaction)
        .await
    }

    /// Converts one claimed row and reports whether this statement converted it.
    ///
    /// `false` means the row was no longer in the legacy tuple when the update
    /// ran, which the caller treats as a reason to abort the batch rather than
    /// to assume its own conversion happened.
    pub(crate) async fn seal_legacy_secret(
        transaction: &mut Transaction<'_, Postgres>,
        key_name: &str,
        stored: &StoredWrite<'_>,
    ) -> Result<bool, sqlx::Error> {
        let key = sqlx::query_file_scalar!(
            "src/sql/internal_secrets/seal_legacy_internal_secret.sql",
            key_name,
            stored.value,
            stored.purpose,
            stored.key_version,
            stored.ciphertext_version
        )
        .fetch_optional(&mut **transaction)
        .await?;
        Ok(key.is_some())
    }

    /// Counts the rows that still hold the legacy plaintext representation.
    pub(crate) async fn count_legacy_secrets(&self) -> Result<i64, sqlx::Error> {
        let remaining: Option<i64> =
            sqlx::query_file_scalar!("src/sql/internal_secrets/count_legacy_internal_secrets.sql")
                .fetch_one(self.pool())
                .await?;
        // `count` always yields exactly one row, so an absent value is a broken
        // statement rather than an empty table, and must not read as "nothing
        // left to seal".
        remaining.ok_or(sqlx::Error::RowNotFound)
    }
}
