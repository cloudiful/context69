//! Startup admission for the one pinned retirement migration.
//!
//! The embedded-enrichment retirement migration refuses to run while ingestion
//! work is still active. A normal upgrade can therefore stop before any worker
//! exists to finish that work. This module decides, from the exact migration
//! ledger and the migration's own guard predicates, whether startup may run the
//! ordinary migration path or must first recover persisted work.
//!
//! It never applies a migration and never writes the ledger. Only the ordinary
//! [`sqlx::migrate::Migrator`] does that, after recovery has stopped every
//! writer.

use anyhow::{Context, Result, anyhow};
use sqlx::PgPool;
use sqlx::migrate::{AppliedMigration, Migrate, MigrateError};

use crate::db::MIGRATOR;

/// The retirement migration this admission is pinned to.
pub const RETIREMENT_VERSION: i64 = 20261017080707;
/// The last migration before it; the prior chain must end exactly here.
pub const PRIOR_VERSION: i64 = 20261016115941;
/// SHA-384 of `migrations/20261017080707_retire_embedded_enrichment.sql`.
///
/// Pinned literally, independently of the compiled migration: a changed file
/// must fail this check instead of silently defining its own expectation.
pub const RETIREMENT_SHA384: &str = "fc74560e226258c94ce00058a418ac8e553c60d39a5a6c3d4dacbd660480c42151e4cc895ae5f4f7f5ccf193792cfd5c";

/// Guard counts read directly from the pinned migration's predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct UpgradeGuardCounts {
    /// Documents whose metadata still carries the retired key.
    pub affected_documents: i64,
    /// Nonterminal items joined with nonterminal parent tasks.
    pub blocking_task_items: i64,
    /// Pending or running durable Docling remote jobs.
    pub blocking_remote_jobs: i64,
}

impl UpgradeGuardCounts {
    /// Whether the pinned migration would raise its active-ingestion guard.
    ///
    /// The guard lives inside the migration's `affected > 0` branch, so an
    /// installation with no affected document never blocks on task work and
    /// must keep the ordinary startup path.
    pub fn blocks_retirement(self) -> bool {
        self.affected_documents > 0
            && (self.blocking_task_items > 0 || self.blocking_remote_jobs > 0)
    }
}

/// Why startup chooses a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeAdmission {
    /// Migrate and start normally.
    Normal,
    /// Serve liveness only while persisted work finishes, then migrate.
    Recovery {
        guards: UpgradeGuardCounts,
        prior_version: i64,
    },
}

/// Classify the database before any migration is applied.
///
/// Any state this build cannot prove is the exact pinned pending retirement
/// returns [`UpgradeAdmission::Normal`]: the ordinary migrator then either
/// succeeds normally or fails closed with its own actionable error. Recovery is
/// never entered on an unknown schema.
pub async fn classify(pool: &PgPool) -> Result<UpgradeAdmission> {
    let mut connection = pool
        .acquire()
        .await
        .context("failed to acquire an upgrade-admission connection")?;

    let dirty = match connection.dirty_version(&MIGRATOR.table_name).await {
        Ok(dirty) => dirty,
        Err(error) if is_missing_ledger(&error) => return Ok(UpgradeAdmission::Normal),
        Err(error) => return Err(anyhow!(error)).context("failed to read the migration ledger"),
    };
    if dirty.is_some() {
        return Ok(UpgradeAdmission::Normal);
    }

    let applied = connection
        .list_applied_migrations(&MIGRATOR.table_name)
        .await
        .context("failed to list applied migrations")?;
    drop(connection);

    let pending = pin_pending_retirement(&applied)?;
    if !pending {
        return Ok(UpgradeAdmission::Normal);
    }

    let guards = guard_counts(pool).await?;
    if !guards.blocks_retirement() {
        return Ok(UpgradeAdmission::Normal);
    }
    Ok(UpgradeAdmission::Recovery {
        guards,
        prior_version: PRIOR_VERSION,
    })
}

/// Read the migration's own guard counts.
pub async fn guard_counts(pool: &PgPool) -> Result<UpgradeGuardCounts> {
    sqlx::query_file_as!(
        UpgradeGuardCounts,
        "src/sql/db/upgrade_admission/guard_counts.sql"
    )
    .fetch_one(pool)
    .await
    .context("failed to count pending-upgrade guard rows")
}

/// Whether the ledger is exactly the complete prior chain plus the one pinned
/// pending retirement migration.
///
/// Returns `Ok(false)` for anything else, including an empty ledger, a missing
/// prior migration, an unknown version, a checksum mismatch, or an already
/// applied retirement.
pub fn pin_pending_retirement(applied: &[AppliedMigration]) -> Result<bool> {
    let expected = expected_retirement_checksum()?;
    let retirement = MIGRATOR
        .iter()
        .find(|migration| migration.version == RETIREMENT_VERSION)
        .ok_or_else(|| {
            anyhow!("the pinned retirement migration is not compiled into this build")
        })?;
    if retirement.checksum.as_ref() != expected.as_slice() {
        return Err(anyhow!(
            "the compiled retirement migration does not match its pinned SHA-384"
        ));
    }
    if MIGRATOR
        .iter()
        .filter(|migration| migration.version > PRIOR_VERSION)
        .count()
        != 1
    {
        return Ok(false);
    }

    let prior: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version <= PRIOR_VERSION)
        .collect();
    if applied.len() != prior.len() {
        return Ok(false);
    }
    for (row, migration) in applied.iter().zip(prior) {
        if row.version != migration.version || row.checksum.as_ref() != migration.checksum.as_ref()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Compare the compiled migration against the independent literal digest.
fn expected_retirement_checksum() -> Result<Vec<u8>> {
    let bytes = RETIREMENT_SHA384.as_bytes();
    if bytes.len() != 96 {
        return Err(anyhow!("the pinned retirement SHA-384 is malformed"));
    }
    let mut checksum = Vec::with_capacity(48);
    for pair in bytes.chunks(2) {
        let [high, low] = pair else {
            return Err(anyhow!("the pinned retirement SHA-384 is malformed"));
        };
        checksum.push((hex_nibble(*high)? << 4) | hex_nibble(*low)?);
    }
    Ok(checksum)
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(anyhow!("the pinned retirement SHA-384 is not hexadecimal")),
    }
}

fn is_missing_ledger(error: &MigrateError) -> bool {
    match error {
        MigrateError::Execute(sqlx::Error::Database(database))
        | MigrateError::ExecuteMigration(sqlx::Error::Database(database), _) => {
            database.code().as_deref() == Some("42P01")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use sqlx::migrate::AppliedMigration;

    use super::{
        PRIOR_VERSION, RETIREMENT_VERSION, UpgradeGuardCounts, expected_retirement_checksum,
        pin_pending_retirement,
    };
    use crate::db::MIGRATOR;

    fn prior_ledger() -> Vec<AppliedMigration> {
        MIGRATOR
            .iter()
            .filter(|migration| migration.version <= PRIOR_VERSION)
            .map(|migration| AppliedMigration {
                version: migration.version,
                checksum: migration.checksum.clone(),
            })
            .collect()
    }

    #[test]
    fn the_pinned_checksum_matches_the_compiled_migration() {
        let expected = expected_retirement_checksum().expect("decode pinned checksum");
        let migration = MIGRATOR
            .iter()
            .find(|migration| migration.version == RETIREMENT_VERSION)
            .expect("retirement migration is compiled");
        assert_eq!(migration.checksum.as_ref(), expected.as_slice());
    }

    #[test]
    fn the_complete_prior_chain_plus_the_pending_retirement_is_admitted() {
        assert!(pin_pending_retirement(&prior_ledger()).expect("pinned ledger"));
    }

    #[test]
    fn an_already_applied_retirement_is_not_admitted() {
        let mut applied = prior_ledger();
        let retirement = MIGRATOR
            .iter()
            .find(|migration| migration.version == RETIREMENT_VERSION)
            .expect("retirement migration is compiled");
        applied.push(AppliedMigration {
            version: retirement.version,
            checksum: retirement.checksum.clone(),
        });
        assert!(!pin_pending_retirement(&applied).expect("applied ledger"));
    }

    #[test]
    fn a_missing_prior_migration_is_not_admitted() {
        let mut applied = prior_ledger();
        applied.pop();
        assert!(!pin_pending_retirement(&applied).expect("short ledger"));
    }

    #[test]
    fn an_unknown_extra_version_is_not_admitted() {
        let mut applied = prior_ledger();
        applied.push(AppliedMigration {
            version: PRIOR_VERSION + 1,
            checksum: b"unknown".to_vec().into(),
        });
        assert!(!pin_pending_retirement(&applied).expect("unknown ledger"));
    }

    #[test]
    fn a_changed_prior_checksum_is_not_admitted() {
        let mut applied = prior_ledger();
        applied[0].checksum = b"changed".to_vec().into();
        assert!(!pin_pending_retirement(&applied).expect("changed ledger"));
    }

    #[test]
    fn an_empty_ledger_is_not_admitted() {
        assert!(!pin_pending_retirement(&[]).expect("empty ledger"));
    }

    #[test]
    fn guard_counts_only_block_when_the_migration_would_check_them() {
        let active = UpgradeGuardCounts {
            affected_documents: 1,
            blocking_task_items: 1,
            blocking_remote_jobs: 0,
        };
        assert!(active.blocks_retirement());
        assert!(
            !UpgradeGuardCounts {
                affected_documents: 0,
                ..active
            }
            .blocks_retirement()
        );
        assert!(
            !UpgradeGuardCounts {
                blocking_task_items: 0,
                blocking_remote_jobs: 0,
                ..active
            }
            .blocks_retirement()
        );
    }
}
