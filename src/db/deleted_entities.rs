//! Entity tombstones (issue #1138 P24).
//!
//! A deleted library file or an explicitly deleted document leaves the search
//! results through its entity identity: the coherent search snapshot reads the
//! tombstoned file and document ids and the index excludes their points by the
//! indexed payload fields. A tombstone therefore consumes no per-point
//! registry budget, and it is permanent — a purge of the physical rows never
//! removes it, and it survives the deletion of the group, the file and the
//! document it names.
//!
//! Every write here runs on a borrowed connection, so the tombstone and the
//! two generation bumps commit with the caller's transaction or roll back
//! with it. The registry epoch is the coherence marker of the search snapshot
//! and the search generation invalidates the cached search responses, so a
//! new tombstone advances both.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::Database;

/// One tombstoned entity.
///
/// Exactly one of `file_id` and `document_id` is set; the table's own check
/// constraint refuses any other shape. `group_id` is the audit value of the
/// group the entity belonged to, never a reference: a tombstone outlives its
/// group.
#[derive(Debug, Clone, PartialEq)]
pub struct DeletedEntity {
    pub id: i64,
    pub file_id: Option<Uuid>,
    pub document_id: Option<i64>,
    pub group_id: Option<i64>,
    pub deleted_at: DateTime<Utc>,
    pub purged_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct DeletedEntityRow {
    id: i64,
    file_id: Option<Uuid>,
    document_id: Option<i64>,
    group_id: Option<i64>,
    deleted_at: DateTime<Utc>,
    purged_at: Option<DateTime<Utc>>,
}

impl From<DeletedEntityRow> for DeletedEntity {
    fn from(row: DeletedEntityRow) -> Self {
        Self {
            id: row.id,
            file_id: row.file_id,
            document_id: row.document_id,
            group_id: row.group_id,
            deleted_at: row.deleted_at,
            purged_at: row.purged_at,
        }
    }
}

impl Database {
    /// Tombstone one deleted library file on the caller's transaction.
    ///
    /// Returns `true` when the tombstone is new. A repeated delete of the same
    /// file inserts nothing and advances neither generation, so the caller
    /// never pays for an exclusion it already owns.
    pub async fn insert_deleted_file_entity_in(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
        group_id: Option<i64>,
    ) -> Result<bool> {
        let inserted = sqlx::query_file_scalar!(
            "src/sql/db/deleted_entities/insert_file_entity.sql",
            file_id,
            group_id
        )
        .fetch_optional(&mut *connection)
        .await
        .context("insert deleted file entity")?;
        if inserted.is_none() {
            return Ok(false);
        }
        self.advance_search_generations_in(connection).await?;
        Ok(true)
    }

    /// Tombstone one explicitly deleted document on the caller's transaction.
    ///
    /// Returns `true` when the tombstone is new. A repeated delete of the same
    /// document inserts nothing and advances neither generation.
    pub async fn insert_deleted_document_entity_in(
        &self,
        connection: &mut PgConnection,
        document_id: i64,
        group_id: Option<i64>,
    ) -> Result<bool> {
        let inserted = sqlx::query_file_scalar!(
            "src/sql/db/deleted_entities/insert_document_entity.sql",
            document_id,
            group_id
        )
        .fetch_optional(&mut *connection)
        .await
        .context("insert deleted document entity")?;
        if inserted.is_none() {
            return Ok(false);
        }
        self.advance_search_generations_in(connection).await?;
        Ok(true)
    }

    /// The tombstone row of one deleted entity.
    ///
    /// The lookup is the identity of the pending cleanup: the row that carries
    /// the `purged_at` observation of the physical purge. Exactly one of the
    /// two ids is set, so exactly one tombstone can match.
    pub async fn deleted_entity_id(
        &self,
        file_id: Option<Uuid>,
        document_id: Option<i64>,
    ) -> Result<Option<i64>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/deleted_entities/read_entity_id.sql",
            file_id,
            document_id
        )
        .fetch_optional(&self.pool)
        .await
        .context("read deleted entity id")?)
    }

    /// The tombstones whose points were not observed as physically gone yet.
    ///
    /// The list is bounded by `limit`; a negative limit is refused rather than
    /// silently turned into an unbounded read.
    pub async fn list_pending_deleted_entities(&self, limit: i64) -> Result<Vec<DeletedEntity>> {
        anyhow::ensure!(
            limit > 0,
            "the pending tombstone list needs a positive bound"
        );
        let rows = sqlx::query_file_as!(
            DeletedEntityRow,
            "src/sql/db/deleted_entities/list_pending.sql",
            limit
        )
        .fetch_all(&self.pool)
        .await
        .context("list pending deleted entities")?;
        Ok(rows.into_iter().map(DeletedEntity::from).collect())
    }

    /// Record that the points of one tombstone were physically observed as
    /// gone.
    ///
    /// The stamp is the observation only: the row stays in the table and the
    /// entity stays excluded from the search snapshot.
    pub async fn mark_deleted_entity_purged_in(
        &self,
        connection: &mut PgConnection,
        id: i64,
    ) -> Result<bool> {
        let purged = sqlx::query_file_scalar!("src/sql/db/deleted_entities/mark_purged.sql", id)
            .fetch_optional(&mut *connection)
            .await
            .context("mark deleted entity purged")?;
        Ok(purged.is_some())
    }

    /// Advance the registry epoch and the search generation together.
    ///
    /// The epoch is the coherence marker the search snapshot re-checks after
    /// every contributing read, and the search generation invalidates the
    /// cached search responses. Both live in the caller's transaction, so a
    /// rollback leaves the snapshot and the cache exactly as they were.
    async fn advance_search_generations_in(&self, connection: &mut PgConnection) -> Result<()> {
        self.lock_search_registry_epoch_in(connection).await?;
        self.bump_search_registry_epoch_in(connection).await?;
        self.bump_search_generation_in(connection).await?;
        Ok(())
    }
}
