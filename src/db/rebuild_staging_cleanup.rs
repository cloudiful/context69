//! Bounded drain of rebuild staging left behind by a completed or cancelled
//! rebuild.
//!
//! A rebuild installs into the live chunk table, so its staged rows are only
//! disposable once the barrier that owned them is gone. Completion and
//! cancellation enqueue their own target in the same transaction that changes
//! the barrier; this storage owns the bounded deletes of one queued target.
//! Remaining rows are the progress, so no lease, retry counter or job table is
//! needed: the existing registry epoch lock serializes the drain against every
//! rebuild writer.

use anyhow::{Context, Result};
use sqlx::PgConnection;

use super::Database;

impl Database {
    /// The next queued staging target that is not the running or failed
    /// barrier's recoverable target.
    pub async fn read_rebuild_staging_cleanup_target_in(
        &self,
        connection: &mut PgConnection,
    ) -> Result<Option<String>> {
        sqlx::query_file_scalar!("src/sql/db/rebuild_staging_cleanup/next_target.sql")
            .fetch_optional(&mut *connection)
            .await
            .context("read the next rebuild staging cleanup target")
    }

    /// Delete at most `limit` staged original chunks of one target.
    pub async fn delete_rebuild_staging_original_batch_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        limit: i64,
    ) -> Result<u64> {
        Ok(sqlx::query_file!(
            "src/sql/db/rebuild_staging_cleanup/delete_original_batch.sql",
            target_collection,
            limit,
        )
        .execute(&mut *connection)
        .await
        .context("delete a staged original chunk batch")?
        .rows_affected())
    }

    /// Delete at most `limit` staged translated chunks of one target.
    pub async fn delete_rebuild_staging_translation_batch_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        limit: i64,
    ) -> Result<u64> {
        Ok(sqlx::query_file!(
            "src/sql/db/rebuild_staging_cleanup/delete_translation_batch.sql",
            target_collection,
            limit,
        )
        .execute(&mut *connection)
        .await
        .context("delete a staged translation chunk batch")?
        .rows_affected())
    }

    /// Delete at most `limit` staged document metadata rows of one target,
    /// only once both staged chunk families are empty.
    pub async fn delete_rebuild_staging_documents_batch_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        limit: i64,
    ) -> Result<u64> {
        Ok(sqlx::query_file!(
            "src/sql/db/rebuild_staging_cleanup/delete_documents_batch.sql",
            target_collection,
            limit,
        )
        .execute(&mut *connection)
        .await
        .context("delete a staged document batch")?
        .rows_affected())
    }

    /// Remove the address row once every staging row family of its target is
    /// empty.
    pub async fn remove_rebuild_staging_cleanup_target_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/db/rebuild_staging_cleanup/remove_target.sql",
            target_collection,
        )
        .execute(&mut *connection)
        .await
        .context("remove the drained staging cleanup target")?
        .rows_affected()
            > 0)
    }
}
