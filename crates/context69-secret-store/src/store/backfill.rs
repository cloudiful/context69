//! Sealing the legacy plaintext rows a deployment still holds.
//!
//! A deployment that has just configured a master key has rows the transition
//! state wrote: raw plaintext, no owning purpose, key version zero. Nothing in
//! the ordinary write path converts them, so a deployment that adds a key would
//! otherwise keep its secrets readable from the table in the clear forever.
//! This module is the conversion, and it runs once at startup before anything
//! resolves a credential.
//!
//! What the run guarantees:
//!
//! * **Nothing is sealed without a key.** The no-key deployment has nothing to
//!   seal *with*, so it reports zero work before it opens a transaction or
//!   issues a statement at all — a legacy plaintext row stays readable exactly
//!   as before, and an unkeyed start never touches the table.
//! * **Only the exact legacy tuple is converted.** A claimed row must still be
//!   plaintext with no owner and key version zero; a row whose markers disagree
//!   is malformed, and the run stops on it instead of guessing.
//! * **Ownership comes from the catalogue, never from the row.** A key name is
//!   resolved through [`SecretPurpose::owning_purpose`], so a value is sealed
//!   under the owner its key already implies. A name the catalogue does not own
//!   stops the run; it is never assigned an owner and never dropped.
//! * **Already-sealed rows are untouched.** They are not claimed, not opened,
//!   and not validated against the configured key, so a row sealed under a
//!   different key keeps its bytes and its timestamps.
//! * **Batched, ordered, and atomic.** One row-locked batch per transaction,
//!   ascending by key, no `SKIP LOCKED`: a competing startup blocks on the first
//!   row of the worklist instead of seeing a false-empty batch and reporting a
//!   sweep that did not happen. A batch either commits whole or leaves the table
//!   exactly as it found it.
//! * **Resumable, and success is measured.** A later run re-selects only what
//!   is still plaintext, and the run reports success only after the remaining
//!   plaintext count is zero — so a row the claim query could not classify can
//!   never be counted as converted.
//!
//! The sealing itself is [`seal_for_storage`], the same function an ordinary
//! write uses, so a converted row is byte-for-byte what that deployment would
//! have written for the same value, under the same framing and AAD.

use std::fmt;

use context69_secret_crypto::SecretCipher;
use sqlx::Postgres;
use tracing::debug;

use crate::{
    database::{
        SecretDatabase, StoredWrite,
        backfill::{LEGACY_BATCH_LIMIT, LegacySecretRow},
    },
    error::SecretStoreError,
    purpose::SecretPurpose,
    rows::{LEGACY_PLAINTEXT_VERSION, seal_for_storage},
    store::SecretStore,
};

/// Bounded counts from one sealing run.
///
/// Two integers and nothing else: no key name, no purpose, no value, so a report
/// is safe to log, to attach to a health response, and to paste into an audit
/// note. After a successful run every row the deployment held is sealed and
/// `sealed` is how many of them this run converted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SealedLegacyReport {
    /// Rows converted and committed by this run.
    pub sealed: usize,
    /// Transactions that converted at least one row.
    pub batches: usize,
}

/// Why a sealing run did not complete.
///
/// Every variant is a category. None carries a key name, a purpose, a value,
/// key material, or the underlying database and cryptographic causes — those
/// carry SQL text and bind values, which is exactly what a startup log line and
/// an error bubbled to an operator must never gain. The category says what the
/// operator has to go and look at; the cause stays out of the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealedLegacyError {
    /// A claimed row's key name is not one the purpose catalogue owns.
    UnownedKey,
    /// A claimed row is not the exact legacy tuple, so it is not convertible.
    MalformedLegacyRow,
    /// A claimed row's key name is not a name a sealed frame could carry.
    InvalidKeyName,
    /// A claimed row could not be sealed.
    SealFailed,
    /// A guarded update did not convert exactly the row it was given, so the row
    /// changed under the run and the batch was refused.
    RowChangedUnderneath,
    /// The database refused the batch.
    DatabaseUnavailable,
    /// Rows were still plaintext after the sweep drained its worklist, so a
    /// writer added legacy rows as the run finished.
    LegacyRowsRemain,
}

impl fmt::Display for SealedLegacyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnownedKey => formatter.write_str(
                "a stored secret's key name is not owned by this build's secret catalogue",
            ),
            Self::MalformedLegacyRow => formatter.write_str(
                "a stored secret is marked legacy plaintext but does not carry the legacy markers",
            ),
            Self::InvalidKeyName => {
                formatter.write_str("a stored secret's key name is not a valid sealed-secret name")
            }
            Self::SealFailed => {
                formatter.write_str("a stored secret could not be sealed under the configured key")
            }
            Self::RowChangedUnderneath => formatter.write_str(
                "a stored secret changed while it was being sealed; the sealing batch was refused",
            ),
            Self::DatabaseUnavailable => {
                formatter.write_str("the database could not complete the legacy secret sealing run")
            }
            Self::LegacyRowsRemain => formatter
                .write_str("stored secrets are still unencrypted after the sealing run finished"),
        }
    }
}

impl std::error::Error for SealedLegacyError {}

impl SecretStore {
    /// Seals every legacy plaintext row this deployment still holds, and reports
    /// how many rows that was.
    ///
    /// A deployment with no master key reports zero without touching the
    /// database: there is nothing to seal with, and the legacy read path stays
    /// exactly as it was.
    ///
    /// # Errors
    ///
    /// [`SealedLegacyError`], stopping at the first batch that could not be
    /// converted. Batches committed before it stay sealed, and the next startup
    /// resumes from the rows that are still plaintext.
    pub async fn seal_legacy_plaintext(&self) -> Result<SealedLegacyReport, SealedLegacyError> {
        // Decided before a transaction is opened, not after: an unkeyed
        // deployment must not claim, count, or write anything.
        let Some(cipher) = self.cipher.as_ref() else {
            return Ok(SealedLegacyReport::default());
        };
        let mut report = SealedLegacyReport::default();
        while let Some(sealed) = self.seal_next_batch(cipher).await? {
            report.sealed += sealed;
            report.batches += 1;
        }
        // Measured, not assumed. A drained worklist says the run saw no
        // plaintext rows; only the count says none are left.
        if self
            .db
            .count_legacy_secrets()
            .await
            .map_err(|_| SealedLegacyError::DatabaseUnavailable)?
            != 0
        {
            return Err(SealedLegacyError::LegacyRowsRemain);
        }
        debug!(
            sealed = report.sealed,
            batches = report.batches,
            "sealed legacy plaintext secrets"
        );
        Ok(report)
    }

    /// Seals the next batch and returns how many rows it converted, or `None`
    /// once the worklist is drained.
    async fn seal_next_batch(
        &self,
        cipher: &SecretCipher,
    ) -> Result<Option<usize>, SealedLegacyError> {
        let mut transaction = self
            .db
            .begin_sealing_batch()
            .await
            .map_err(|_| SealedLegacyError::DatabaseUnavailable)?;
        let rows = SecretDatabase::claim_legacy_secrets(&mut transaction, LEGACY_BATCH_LIMIT)
            .await
            .map_err(|_| SealedLegacyError::DatabaseUnavailable)?;
        if rows.is_empty() {
            transaction
                .commit()
                .await
                .map_err(|_| SealedLegacyError::DatabaseUnavailable)?;
            return Ok(None);
        }
        // Every row stays locked from here to the commit, so a value is sealed
        // against exactly the bytes that were claimed.
        let sealed = rows.len();
        for row in rows {
            seal_claimed_row(&mut transaction, cipher, &row).await?;
        }
        // Counted after the commit, so a report never claims a row the table
        // does not hold.
        transaction
            .commit()
            .await
            .map_err(|_| SealedLegacyError::DatabaseUnavailable)?;
        Ok(Some(sealed))
    }
}

/// Seals one claimed row inside its batch's transaction.
///
/// A refusal here aborts the batch, so the transaction rolls back whole and the
/// rows already converted in it stay exactly as they were.
async fn seal_claimed_row(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    cipher: &SecretCipher,
    row: &LegacySecretRow,
) -> Result<(), SealedLegacyError> {
    if !is_convertible_legacy_tuple(row) {
        return Err(SealedLegacyError::MalformedLegacyRow);
    }
    let purpose = SecretPurpose::owning_purpose(&row.key).ok_or(SealedLegacyError::UnownedKey)?;
    // The ordinary write path's sealing, so a converted row is indistinguishable
    // from one this deployment had written with the key all along.
    let stored =
        seal_for_storage(Some(cipher), purpose, &row.key, &row.value).map_err(map_seal_error)?;
    let converted =
        SecretDatabase::seal_legacy_secret(transaction, &row.key, &StoredWrite::from(&stored))
            .await
            .map_err(|_| SealedLegacyError::DatabaseUnavailable)?;
    if converted {
        Ok(())
    } else {
        Err(SealedLegacyError::RowChangedUnderneath)
    }
}

/// Whether a claimed row is exactly the legacy tuple this run converts.
fn is_convertible_legacy_tuple(row: &LegacySecretRow) -> bool {
    row.ciphertext_version == LEGACY_PLAINTEXT_VERSION
        && row.purpose.is_none()
        && row.key_version == LEGACY_PLAINTEXT_VERSION
}

/// Collapses a sealing failure to its category, dropping the cipher error.
///
/// The rejected input is a value and a name, so it never travels further.
fn map_seal_error(error: SecretStoreError) -> SealedLegacyError {
    match error {
        SecretStoreError::InvalidKeyName => SealedLegacyError::InvalidKeyName,
        _ => SealedLegacyError::SealFailed,
    }
}
