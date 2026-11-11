//! Durable per-file latest-state marker of the canonical metadata mirror.
//!
//! One row names a library file whose canonical metadata still needs a vector
//! payload mirror. Every enqueue advances the marker version and resets the
//! retry state, so a pending marker always describes the newest publication.
//! The mirror worker holds a per-file session advisory lock on a dedicated
//! pooled connection, rereads the current canonical payloads on that
//! connection, and completes only the exact version it observed: a completion
//! that matches no row means a newer enqueue already exists, and the marker is
//! never deleted on behalf of the stale version.

use anyhow::{Result, anyhow};
use sqlx::postgres::PgConnection;
use sqlx::{FromRow, Postgres, pool::PoolConnection};
use tracing::warn;
use uuid::Uuid;

use super::Database;

/// One pending mirror marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryMetadataMirrorMarker {
    pub file_id: Uuid,
    pub version: i64,
    pub attempts: i32,
}

#[derive(FromRow)]
struct LibraryMetadataMirrorMarkerRow {
    file_id: Uuid,
    version: i64,
    attempts: i32,
}

impl From<LibraryMetadataMirrorMarkerRow> for LibraryMetadataMirrorMarker {
    fn from(row: LibraryMetadataMirrorMarkerRow) -> Self {
        Self {
            file_id: row.file_id,
            version: row.version,
            attempts: row.attempts,
        }
    }
}

/// The dedicated connection that holds one file's session advisory lock.
///
/// The lock is a session-level advisory lock, so it spans the canonical reread
/// and the external payload send of one candidate. A dropped guard ends the
/// session instead of returning the connection to the pool with the lock still
/// held, exactly like the startup handover lock.
pub struct LibraryMetadataMirrorLock {
    connection: Option<PoolConnection<Postgres>>,
    file_id: Uuid,
}

impl LibraryMetadataMirrorLock {
    /// The connection that holds the advisory lock. Never acquire another
    /// pooled connection while this borrow is alive.
    pub fn connection(&mut self) -> &mut PgConnection {
        self.connection
            .as_mut()
            .map(|connection| &mut **connection)
            .expect("an unreleased mirror lock keeps its connection")
    }

    /// Releases the lock explicitly and returns the connection to the pool.
    pub async fn release(mut self) {
        let unlocked = match sqlx::query_file_scalar!(
            "src/sql/db/library_metadata_mirror/session_unlock.sql",
            self.file_id
        )
        .fetch_one(self.connection())
        .await
        {
            Ok(unlocked) => unlocked,
            Err(error) => {
                warn!(
                    file_id = %self.file_id,
                    error = %error,
                    "the metadata mirror lock could not be released; the session ends with its connection"
                );
                // `self` still owns the connection, so dropping it detaches and
                // ends the session instead of returning a possibly locked one.
                return;
            }
        };
        if !unlocked {
            warn!(
                file_id = %self.file_id,
                "the metadata mirror lock was not held by the session that took it"
            );
        }
        // Only a confirmed server-side unlock returns the session to the pool.
        let connection = self
            .connection
            .take()
            .expect("an unreleased mirror lock keeps its connection");
        drop(connection);
    }
}

impl Drop for LibraryMetadataMirrorLock {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            // The session still holds the lock: ending the session releases it.
            drop(connection.detach());
        }
    }
}

impl Database {
    /// Enqueue one file in the caller's canonical metadata transaction.
    ///
    /// A pending marker is advanced, never skipped: the newest publication
    /// always owns the completion, and a later worker pass rereads the current
    /// payloads instead of replaying an older copy.
    pub async fn enqueue_library_metadata_mirror_in(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
    ) -> Result<()> {
        sqlx::query_file!("src/sql/db/library_metadata_mirror/enqueue.sql", file_id)
            .execute(connection)
            .await?;
        Ok(())
    }

    /// Up to `limit` due markers, in retry order.
    pub async fn read_due_library_metadata_mirrors(
        &self,
        limit: i64,
    ) -> Result<Vec<LibraryMetadataMirrorMarker>> {
        let rows = sqlx::query_file_as!(
            LibraryMetadataMirrorMarkerRow,
            "src/sql/db/library_metadata_mirror/read_due.sql",
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// The marker of one file, reread under its session lock.
    pub async fn read_library_metadata_mirror_in(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
    ) -> Result<Option<LibraryMetadataMirrorMarker>> {
        let row = sqlx::query_file_as!(
            LibraryMetadataMirrorMarkerRow,
            "src/sql/db/library_metadata_mirror/read_one.sql",
            file_id
        )
        .fetch_optional(connection)
        .await?;
        Ok(row.map(Into::into))
    }

    /// Delete the marker only while it still names `version`.
    ///
    /// `false` means a newer enqueue exists and the marker must stay pending.
    pub async fn complete_library_metadata_mirror_in(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
        version: i64,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/library_metadata_mirror/complete.sql",
            file_id,
            version
        )
        .execute(connection)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Delay only the observed version; a newer enqueue keeps its own state.
    pub async fn backoff_library_metadata_mirror_in(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
        version: i64,
        retry_after_seconds: i32,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/library_metadata_mirror/backoff.sql",
            file_id,
            version,
            retry_after_seconds
        )
        .execute(connection)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Try to take the per-file session advisory lock on a dedicated
    /// connection. `None` means another worker or instance owns this file.
    pub async fn try_lock_library_metadata_mirror(
        &self,
        file_id: Uuid,
    ) -> Result<Option<LibraryMetadataMirrorLock>> {
        let connection = self
            .pool()
            .acquire()
            .await
            .map_err(|error| anyhow!("acquire the metadata mirror session: {error}"))?;
        // The guard owns the connection before the advisory statement is
        // submitted, so an aborted acquisition detaches the session instead of
        // returning a possibly locked connection to the pool.
        let mut guard = LibraryMetadataMirrorLock {
            connection: Some(connection),
            file_id,
        };
        let locked = match sqlx::query_file_scalar!(
            "src/sql/db/library_metadata_mirror/session_lock.sql",
            file_id
        )
        .fetch_one(guard.connection())
        .await
        {
            Ok(locked) => locked,
            Err(error) => {
                // The outcome of the lock statement is unknown: dropping the
                // guard ends the session with its connection.
                return Err(error.into());
            }
        };
        if locked {
            Ok(Some(guard))
        } else {
            // Only a known-safe, unlocked session returns to the pool.
            let connection = guard
                .connection
                .take()
                .expect("an unreleased mirror lock keeps its connection");
            drop(connection);
            Ok(None)
        }
    }
}
