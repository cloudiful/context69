//! The single session advisory lock that serialises destructive backend runs.
//!
//! The lock is held by one dedicated connection for the whole
//! reset/migrate/test/cleanup sequence, so a second runner can never reset the
//! target while tests are active. Because the lock is session-scoped, closing
//! the connection releases it; [`SessionLock::release`] is therefore an
//! explicit courtesy and `Drop` remains the actual guarantee.

use sqlx::{Connection, postgres::PgConnection};

use super::preflight;
use crate::config::Endpoint;

/// Namespace mixed into the lock key so the runner never contends with an
/// unrelated advisory lock.
pub const LOCK_NAMESPACE: &str = "context69-backend-test-runner";

/// The blocking acquire never yields an unheld lock; this exists only so the
/// impossible case has a named, secret-free error.
#[derive(Debug, Clone, Copy)]
pub struct LockNotHeld;

impl std::fmt::Display for LockNotHeld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the blocking advisory lock did not return a held lock")
    }
}

impl std::error::Error for LockNotHeld {}

/// The two-key advisory lock identifier for one database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockKeys {
    pub namespace: i32,
    pub database_oid: i32,
}

impl LockKeys {
    /// Derive the lock keys from the namespace text and the target database's
    /// own oid.
    #[must_use]
    pub fn new(database_oid: i32) -> Self {
        Self {
            namespace: fnv1a32(LOCK_NAMESPACE.as_bytes()),
            database_oid,
        }
    }

    /// Whether two lock identifiers can contend for the same lock.
    #[must_use]
    pub fn same_lock(&self, other: &Self) -> bool {
        self == other
    }
}

/// A held session lock. Dropping it closes the connection, which releases the
/// lock.
pub struct SessionLock {
    connection: PgConnection,
    keys: LockKeys,
}

impl std::fmt::Debug for SessionLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionLock")
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl SessionLock {
    /// Take the lock on a fresh dedicated session, waiting for any runner that
    /// already holds it.
    ///
    /// # Errors
    ///
    /// Returns the sqlx error when the connection or the lock statement fails.
    pub async fn acquire(endpoint: &Endpoint) -> Result<Self, sqlx::Error> {
        Self::take(endpoint, false)
            .await?
            .ok_or(sqlx::Error::Configuration(Box::new(LockNotHeld)))
    }

    /// Take the lock only if it is free.
    ///
    /// # Errors
    ///
    /// Returns the sqlx error when the connection or the lock statement fails.
    pub async fn try_acquire(endpoint: &Endpoint) -> Result<Option<Self>, sqlx::Error> {
        Self::take(endpoint, true).await
    }

    /// Release the lock explicitly and close the session.
    ///
    /// # Errors
    ///
    /// Returns the sqlx error when the unlock statement fails.
    pub async fn release(mut self) -> Result<(), sqlx::Error> {
        sqlx::query(include_str!("../sql/advisory_unlock.sql"))
            .bind(self.keys.namespace)
            .bind(self.keys.database_oid)
            .execute(&mut self.connection)
            .await?;
        self.connection.close().await
    }

    async fn take(endpoint: &Endpoint, non_blocking: bool) -> Result<Option<Self>, sqlx::Error> {
        let options = preflight::connect_options(endpoint)?;
        let mut connection = PgConnection::connect_with(&options).await?;

        let keys: (i32, i32) = sqlx::query_as(include_str!("../sql/advisory_lock_keys.sql"))
            .fetch_one(&mut connection)
            .await?;
        let keys = LockKeys::new(keys.1);

        if non_blocking {
            let (acquired,): (bool,) = sqlx::query_as(include_str!("../sql/try_advisory_lock.sql"))
                .bind(keys.namespace)
                .bind(keys.database_oid)
                .fetch_one(&mut connection)
                .await?;
            if !acquired {
                return Ok(None);
            }
        } else {
            sqlx::query(include_str!("../sql/advisory_lock.sql"))
                .bind(keys.namespace)
                .bind(keys.database_oid)
                .execute(&mut connection)
                .await?;
        }

        Ok(Some(Self { connection, keys }))
    }
}

/// Whether a runner may start given the outcome of its lock attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The runner holds the lock and owns the target for this sequence.
    Proceed,
    /// Another runner holds the lock; this one must not reset or test.
    RefuseAnotherRunnerActive,
}

impl std::fmt::Display for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proceed => f.write_str("proceed"),
            Self::RefuseAnotherRunnerActive => {
                f.write_str("another runner holds the target database lock")
            }
        }
    }
}

/// Turn a lock attempt into an admission decision.
#[must_use]
pub fn admit(held: Option<&SessionLock>) -> Admission {
    if held.is_some() {
        Admission::Proceed
    } else {
        Admission::RefuseAnotherRunnerActive
    }
}

fn fnv1a32(bytes: &[u8]) -> i32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    i32::try_from(hash & 0x7fff_ffff).unwrap_or_default()
}
