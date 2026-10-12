//! The destructive half of the runner, reachable only after a passing
//! preflight.
//!
//! Two independent gates guard this code. [`ResetAuthorization`] requires a
//! [`crate::runner::preflight::ResetDecision`] whose `allowed` flag is true, and
//! additionally requires the caller to hold the recorded security-checkpoint
//! approval. Neither can be satisfied by a missing configuration, so an absent
//! approval leaves reset unreachable.
//!
//! Even with both gates open the reset stays inside its ownership: it drops and
//! recreates the one schema Context69 owns, and clears ledger rows only after
//! the preflight matched every one of them against this build's migrations.

use context69_db_schema::MIGRATOR;
use sqlx::postgres::PgPool;

use crate::config::Endpoint;

use super::preflight::{self, PreflightObservation, ResetDecision};

/// Drop and recreate the one schema Context69 owns. Loaded like a fixture so no
/// destructive SQL lives in a Rust string literal.
///
/// Used on its own when the preflight found the sqlx ledger inside the owned
/// schema, because dropping the schema then removes the ledger with it.
pub const RESET_SQL: &str = include_str!("../sql/owned_schema_reset.sql");

/// Clears only the ledger rows this build's migrations produced.
///
/// The ledger lives in `public`, which the runner does not own, so the pairs are
/// bound rather than the table being emptied. Anything unrecognised in that
/// ledger is a preflight refusal, so reaching here means every row matches.
pub const CLEAR_VERIFIED_LEDGER_SQL: &str = include_str!("../sql/clear_verified_public_ledger.sql");

/// Proof that both the safety guards were satisfied before any write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetAuthorization {
    decision: ResetDecision,
    checkpoint_approved: bool,
}

impl ResetAuthorization {
    /// Combine the preflight verdict with the recorded checkpoint approval.
    #[must_use]
    pub fn new(decision: ResetDecision, checkpoint_approved: bool) -> Self {
        Self {
            decision,
            checkpoint_approved,
        }
    }

    /// Whether this authorization actually permits a reset.
    #[must_use]
    pub fn permits_reset(&self) -> bool {
        self.decision.allowed && self.checkpoint_approved
    }
}

/// Whether the stale ledger rows live outside the owned schema and so have to be
/// cleared on their own.
///
/// The location is observed, never assumed: a ledger inside the owned schema
/// goes away with the schema, while a verified `public` ledger keeps its rows.
#[must_use]
pub fn clears_ledger_separately(observation: &PreflightObservation) -> bool {
    matches!(
        observation.ledgers.as_slice(),
        [ledger] if ledger.namespace == preflight::PUBLIC_SCHEMA
    )
}

/// Drop, recreate, and clear the stale ledger on the configured target.
///
/// # Errors
///
/// Returns [`ResetRefused`] unless the authorization permits a reset, and the
/// sqlx error when the schema statements fail.
pub async fn reset_owned_schema(
    endpoint: &Endpoint,
    authorization: ResetAuthorization,
    observation: &PreflightObservation,
) -> Result<(), ResetError> {
    if !authorization.permits_reset() {
        return Err(ResetRefused {
            preflight_allowed: authorization.decision.allowed,
            checkpoint_approved: authorization.checkpoint_approved,
        }
        .into());
    }

    let pool = PgPool::connect_with(preflight::connect_options(endpoint)?).await?;
    sqlx::raw_sql(RESET_SQL).execute(&pool).await?;

    // A shared ledger is never emptied: only the pairs this build ships are
    // removed, and the preflight has already refused if it found anything else.
    if clears_ledger_separately(observation) {
        let migrations = preflight::repository_migrations();
        let versions = migrations
            .iter()
            .map(|(version, _)| *version)
            .collect::<Vec<_>>();
        let checksums = migrations
            .iter()
            .map(|(_, checksum)| checksum.clone())
            .collect::<Vec<_>>();
        sqlx::query(CLEAR_VERIFIED_LEDGER_SQL)
            .bind(versions)
            .bind(checksums)
            .execute(&pool)
            .await?;
    }

    pool.close().await;
    Ok(())
}

/// Apply the Context69 migrations to a freshly reset schema.
///
/// # Errors
///
/// Returns the migrator error when a migration cannot be applied.
pub async fn initialize_schema(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

/// The reset was not authorized; nothing was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetRefused {
    pub preflight_allowed: bool,
    pub checkpoint_approved: bool,
}

impl std::fmt::Display for ResetRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "reset refused: preflight_allowed={}, checkpoint_approved={}",
            self.preflight_allowed, self.checkpoint_approved
        )
    }
}

/// Any failure of the destructive sequence.
#[derive(Debug)]
pub enum ResetError {
    Refused(ResetRefused),
    Database(sqlx::Error),
}

impl From<ResetRefused> for ResetError {
    fn from(refused: ResetRefused) -> Self {
        Self::Refused(refused)
    }
}

impl From<sqlx::Error> for ResetError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl std::fmt::Display for ResetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refused) => write!(f, "{refused}"),
            Self::Database(_) => f.write_str("the owned-schema reset did not complete"),
        }
    }
}

impl std::error::Error for ResetError {}

/// The one schema this runner is allowed to drop and recreate.
pub use preflight::OWNED_SCHEMA;
