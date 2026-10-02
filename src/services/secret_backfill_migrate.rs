//! The reversible-secret backfill's migration step: seal, verify, then clean up.
//!
//! One item, in one fixed order. The store write commits first, the value is read back
//! and compared, and only then may a legacy column be nulled or a reference
//! repointed — so a value is never dropped before the store holds it. Every failure
//! names the owning purpose or the legacy column and never a value, a key name, or a
//! connection name.
//!
//! Budget and ordering live here too, because they are properties of the *change*: an
//! item that is already finished, and one with nothing to migrate, costs no budget and
//! never stops a run, so `limit_reached` means a change is genuinely left rather than
//! that a retained category or an empty column was examined.

use anyhow::{Context, Result};

use crate::{
    db::Database,
    services::{
        secret_backfill::{BackfillOptions, BackfillReport},
        secret_backfill_items::{Disposition, LegacyItem},
        secret_store::{SecretPurpose, SecretStore},
        settings::secrets::secret_error,
    },
};

/// Moves one legacy item into the store and then does its legacy cleanup.
///
/// Returns `false` once the run's budget of changes is spent.
pub(super) async fn migrate(
    db: &Database,
    store: &SecretStore,
    options: &BackfillOptions,
    budget: &mut usize,
    report: &mut BackfillReport,
    item: &LegacyItem,
) -> Result<bool> {
    report.scanned += 1;
    let Some(value) = item.value.as_deref() else {
        report.absent += 1;
        return Ok(true);
    };
    let purpose = item.purpose;
    // A legacy column is finished only when the store already holds exactly the same
    // bytes: a row that merely exists may be a newer credential, so the legacy value is
    // re-sealed rather than assumed equal. A store row that is itself legacy has
    // nothing sealed to compare against, so it is always rewritten.
    let stored = store
        .get(purpose, &item.key_name)
        .await
        .with_context(|| format!("{purpose} stored value could not be read"))?;
    let sealed = item.from_legacy_column && stored.is_some_and(|stored| stored.expose() == value);
    if sealed && !item.needs_cleanup() {
        report.already_sealed += 1;
        return Ok(true);
    }
    if *budget == 0 {
        report.limit_reached = true;
        return Ok(false);
    }
    *budget -= 1;
    if sealed {
        report.already_sealed += 1;
    } else {
        if options.apply {
            store
                .write(purpose, &item.key_name, value)
                .await
                .map_err(|error| secret_error(purpose, error))?;
            verify(store, purpose, &item.key_name, value).await?;
        }
        // Counted only once the write committed and read back, so a stopped run
        // never claims a seal it did not make. In a dry run this is the prediction.
        report.sealed += 1;
    }
    match &item.disposition {
        Disposition::Clear(column) => {
            if options.apply {
                let affected = column.clear(db).await.with_context(|| {
                    format!("{purpose} legacy {} could not be cleared", column.column())
                })?;
                if affected != 1 {
                    return Err(anyhow::anyhow!(
                        "{purpose} legacy {} clear affected {affected} rows, expected one",
                        column.column()
                    ));
                }
            }
            report.cleared += 1;
        }
        Disposition::Retain => report.retained += 1,
        Disposition::SourceReference { name, .. } => {
            if options.apply {
                let matched = db
                    .set_source_connection_database_url_secret_key(name, &item.key_name)
                    .await
                    .with_context(|| {
                        format!("{purpose} source connection reference could not be repointed")
                    })?;
                if !matched {
                    return Err(anyhow::anyhow!(
                        "{purpose} was sealed but no source connection reference was updated"
                    ));
                }
            }
            report.referenced += 1;
        }
    }
    Ok(true)
}

/// Confirms a sealed value reads back exactly as it was written. A store this
/// deployment cannot open, a row a concurrent run removed, and a frame that does not
/// authenticate all stop the run instead of being reported as migrated. The comparison
/// never leaves this function: the error names the purpose, never the value.
async fn verify(
    store: &SecretStore,
    purpose: SecretPurpose,
    key_name: &str,
    value: &[u8],
) -> Result<()> {
    let stored = store
        .get(purpose, key_name)
        .await
        .map_err(|error| secret_error(purpose, error))?;
    if stored.is_none_or(|stored| stored.expose() != value) {
        return Err(anyhow::anyhow!(
            "{purpose} sealed value did not read back as it was written; its legacy value was kept"
        ));
    }
    Ok(())
}
