//! The manual reversible-secret backfill (issue #681 work unit 4A3b-6).
//!
//! A deployment that kept its reversible credentials in plaintext columns, or in
//! the store's own legacy representation, needs them sealed under their purpose
//! before those columns can be removed. This mode is that migration: manual,
//! read-only by default, bounded, resumable, and idempotent per category. No startup
//! path runs it — nothing here happens unless an operator asks for it, through
//! [`crate::services::maintenance::backfill_secrets`].
//!
//! One run **refuses** to run at all without a configured master key, before the first
//! row is read: an unencrypted store cannot open a sealed row it is asked to
//! inventory, and could not seal a legacy value either, so a run under it could only
//! report what it cannot substantiate. It then **reads** each legacy value raw — through
//! the projections its own consumers read, and through the store's key list for rows
//! still in the legacy representation; a stored value is never the *source* of a
//! migration. It **seals** that value under its catalogue purpose and key name and
//! reads it back, and only a write that commits *and* verifies is followed by a legacy
//! cleanup. Finally it **clears** the four standalone nullable credential columns, or
//! **references** the sealed value from the source connection whose legacy DSN stays
//! put.
//!
//! The store write always precedes the clear or the reference, so a value is never
//! dropped before the store holds it, and the first failure stops the run with every
//! finished item intact — so re-running resumes rather than repairs. A run reports
//! bounded counts and nothing else: no value, no key name, no DSN, no connection name.
//!
//! This module owns that public shape — the controls, the report, the failure, and the
//! order the worklist is walked in. The worklist itself is built by
//! [`crate::services::secret_backfill_items`] and migrated by
//! [`crate::services::secret_backfill_migrate`].

use std::fmt;

use anyhow::{Context, Result};

use crate::{
    db::Database,
    services::{
        secret_backfill_items::{singleton_items, source_item, unversioned_item},
        secret_backfill_migrate::migrate,
        secret_store::SecretStore,
    },
};

/// How many legacy items one run may change when `--limit` is not given. Bounded by
/// default, so a large deployment is migrated in batches and every run terminates. A
/// run that reports `limit_reached` has work left; the mode is run again until a run
/// reports none.
pub const DEFAULT_LIMIT: usize = 500;

/// One run's controls, as the mode's arguments express them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillOptions {
    /// Whether the run may commit. Without it, the run reads and reports only.
    pub apply: bool,
    /// How many legacy items this run may change.
    pub limit: usize,
}

impl Default for BackfillOptions {
    /// A read-only inventory bounded by [`DEFAULT_LIMIT`].
    fn default() -> Self {
        Self {
            apply: false,
            limit: DEFAULT_LIMIT,
        }
    }
}

impl BackfillOptions {
    /// Parses the mode's arguments: `--apply` is the only way to commit and
    /// `--limit <count>` the only way to bound a run. Anything else is refused
    /// rather than ignored, so a mistyped flag can neither turn a rehearsal into a
    /// write nor the reverse.
    ///
    /// # Errors
    ///
    /// An unsupported argument, a `--limit` with no value or a non-count value, and
    /// a count of zero — which would make a run do nothing while reporting that
    /// there was nothing to do.
    pub fn parse<I>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = String>,
    {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--apply" => options.apply = true,
                "--limit" => {
                    options.limit = args
                        .next()
                        .context("--limit needs the number of items this run may change")?
                        .parse()
                        .context("--limit needs a positive item count")?;
                    if options.limit == 0 {
                        return Err(anyhow::anyhow!("--limit must be greater than 0"));
                    }
                }
                other => {
                    return Err(anyhow::anyhow!(
                        "unsupported backfill argument {other}; expected --apply or --limit <count>"
                    ));
                }
            }
        }
        Ok(options)
    }
}

/// Bounded counts from one run. No field can hold a value, a key name, or a
/// connection name, so the report is safe to log and to paste into an audit note.
/// `sealed`, `cleared`, and `referenced` describe what the run committed — and in a
/// dry run, what the same worklist under the same limit would have committed, which
/// is what makes the inventory usable as a rehearsal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Legacy items examined.
    pub scanned: usize,
    /// Items with no legacy value to migrate, including a blank one.
    pub absent: usize,
    /// Items whose stored row already held exactly the legacy value.
    pub already_sealed: usize,
    /// Encrypted writes committed and verified, or that `--apply` would commit.
    pub sealed: usize,
    /// Legacy credential columns nulled, or that `--apply` would null.
    pub cleared: usize,
    /// Source references written or repaired, or that `--apply` would write.
    pub referenced: usize,
    /// Singleton items whose legacy value stays until the column removal.
    pub retained: usize,
    /// Store rows in the legacy representation that the catalogue owns no purpose
    /// for, and that were therefore left untouched.
    pub unmapped: usize,
    /// Whether the limit stopped the run with a change still listed.
    pub limit_reached: bool,
}

impl fmt::Display for BackfillReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "scanned={} absent={} already_sealed={} sealed={} cleared={} referenced={} \
             retained={} unmapped={} limit_reached={}",
            self.scanned,
            self.absent,
            self.already_sealed,
            self.sealed,
            self.cleared,
            self.referenced,
            self.retained,
            self.unmapped,
            self.limit_reached
        )
    }
}

/// Why one run stopped, with the counts it reached before stopping. The reason names
/// the purpose or the column that could not be migrated and never carries a value, a
/// key name, or a connection name.
#[derive(Debug)]
pub struct BackfillFailure {
    /// Counts from the part of the worklist that did finish.
    pub report: BackfillReport,
    /// The first refusal, read, write, clear, reference, or verification failure.
    pub error: anyhow::Error,
}

impl fmt::Display for BackfillFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "secret backfill stopped: {}", self.error)
    }
}

impl std::error::Error for BackfillFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

/// Migrates the deployment's legacy reversible credentials into the shared store.
///
/// # Errors
///
/// [`BackfillFailure`] at the first refusal, read, write, clear, reference update, or
/// round-trip verification failure, carrying the counts reached up to that point.
/// Nothing is rolled back: every item before the failure is finished, and the failing
/// one keeps its legacy value, so the run is resumable.
pub async fn run(
    db: &Database,
    store: &SecretStore,
    options: &BackfillOptions,
) -> Result<BackfillReport, BackfillFailure> {
    let mut report = BackfillReport::default();
    match execute(db, store, options, &mut report).await {
        Ok(()) => Ok(report),
        Err(error) => Err(BackfillFailure { report, error }),
    }
}

/// Walks the worklist in one fixed order, bounded by the run's limit.
async fn execute(
    db: &Database,
    store: &SecretStore,
    options: &BackfillOptions,
    report: &mut BackfillReport,
) -> Result<()> {
    if !store.is_encrypted() {
        return Err(anyhow::anyhow!(
            "the reversible-secret backfill needs a configured master_key (app.master_secret): an \
             unencrypted store cannot open what it is asked to inventory, and could not seal a \
             legacy value anyway"
        ));
    }
    // One fixed order — the store's own legacy rows, then the singleton columns,
    // then source connections by name — so a stopped run always resumes from the same
    // place and two reports are comparable. The store rows come first because a
    // plaintext row can share a singleton key with a legacy column: sealing it before
    // that column may be cleared keeps the seal literal, whatever either side holds.
    let mut budget = options.limit;
    for key_name in store.list_unversioned_keys().await? {
        // An unmapped key is counted and left exactly as it is: sealing it under a
        // guessed purpose would bind one owner's value to another owner's key.
        let Some(item) = unversioned_item(store, &key_name).await? else {
            report.scanned += 1;
            report.unmapped += 1;
            continue;
        };
        if !migrate(db, store, options, &mut budget, report, &item).await? {
            return Ok(());
        }
    }
    for item in singleton_items(db).await? {
        if !migrate(db, store, options, &mut budget, report, &item).await? {
            return Ok(());
        }
    }
    for connection in db.list_source_connections().await? {
        let item = source_item(&connection)?;
        if !migrate(db, store, options, &mut budget, report, &item).await? {
            return Ok(());
        }
    }
    Ok(())
}
