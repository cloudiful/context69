use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Alias, ColumnRef, Expr, ExprTrait, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;
use crate::db::entity::{library_files, library_legacy_object_cleanup, library_storage_objects};

/// The cleanup eligibility of one legacy cleanup record.
///
/// The comparison runs against the server clock, so the column is compared as
/// an expression instead of a bound value.
fn cleanup_eligible_now() -> Expr {
    Expr::col(ColumnRef::from((
        Alias::new("library_legacy_object_cleanup"),
        Alias::new("cleanup_eligible_at"),
    )))
    .lte(Expr::current_timestamp())
}

#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
pub struct StorageObjectRecord {
    pub id: Uuid,
    pub group_id: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_backend: String,
    pub object_key: String,
    pub staging_lease_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DeletedStorageObject {
    pub object_key: String,
    pub storage_backend: String,
}

/// One open legacy old-key cleanup record selected by the cleanup phase.
/// `old_storage_backend` is `None` for rows recorded before migration 0024;
/// such rows have an unknown backend and must never be physically deleted.
#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
pub struct LegacyCleanupRow {
    pub id: i64,
    pub group_id: i64,
    pub file_id: Uuid,
    pub old_key: String,
    pub old_storage_backend: Option<String>,
}

/// Grouped arguments for staging a storage object on a connection.
#[derive(Debug, Clone)]
pub struct UpsertStagedStorageObjectRequest<'a> {
    pub id: Uuid,
    pub group_id: i64,
    pub sha256: &'a str,
    pub size_bytes: i64,
    pub storage_backend: &'a str,
    pub object_key: &'a str,
    pub staging_lease_until: DateTime<Utc>,
}

/// Grouped arguments for upserting a storage object on a connection.
#[derive(Debug, Clone)]
pub struct UpsertStorageObjectRequest<'a> {
    pub id: Uuid,
    pub group_id: i64,
    pub sha256: &'a str,
    pub size_bytes: i64,
    pub storage_backend: &'a str,
    pub object_key: &'a str,
}

impl LibraryStore {
    pub async fn get_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        sha256: &str,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object.sql",
            group_id,
            sha256
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn upsert_staged_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        request: UpsertStagedStorageObjectRequest<'_>,
    ) -> Result<StorageObjectRecord> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/upsert_staged_storage_object.sql",
            request.id,
            request.group_id,
            request.sha256,
            request.size_bytes,
            request.storage_backend,
            request.object_key,
            request.staging_lease_until
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn upsert_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        request: UpsertStorageObjectRequest<'_>,
    ) -> Result<StorageObjectRecord> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/upsert_storage_object.sql",
            request.id,
            request.group_id,
            request.sha256,
            request.size_bytes,
            request.storage_backend,
            request.object_key
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn link_file_storage_object(
        &self,
        file_id: Uuid,
        object_id: Uuid,
        object_key: &str,
    ) -> Result<()> {
        library_files::Entity::update_many()
            .col_expr(library_files::Column::StorageObjectId, Expr::val(object_id))
            .col_expr(
                library_files::Column::StorageRelPath,
                Expr::val(object_key.to_owned()),
            )
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::Id.eq(file_id))
            .exec(self.db.orm())
            .await?;
        Ok(())
    }

    pub async fn get_storage_object(
        &self,
        group_id: i64,
        sha256: &str,
    ) -> Result<Option<StorageObjectRecord>> {
        let row = library_storage_objects::Entity::find()
            .select_only()
            .column(library_storage_objects::Column::Id)
            .column(library_storage_objects::Column::GroupId)
            .column(library_storage_objects::Column::Sha256)
            .column(library_storage_objects::Column::SizeBytes)
            .column(library_storage_objects::Column::StorageBackend)
            .column(library_storage_objects::Column::ObjectKey)
            .column(library_storage_objects::Column::StagingLeaseUntil)
            .filter(library_storage_objects::Column::GroupId.eq(group_id))
            .filter(library_storage_objects::Column::Sha256.eq(sha256))
            .into_model::<StorageObjectRecord>()
            .one(self.db.orm())
            .await?;
        Ok(row)
    }

    pub async fn get_storage_object_by_id(
        &self,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        let row = library_storage_objects::Entity::find_by_id(object_id)
            .select_only()
            .column(library_storage_objects::Column::Id)
            .column(library_storage_objects::Column::GroupId)
            .column(library_storage_objects::Column::Sha256)
            .column(library_storage_objects::Column::SizeBytes)
            .column(library_storage_objects::Column::StorageBackend)
            .column(library_storage_objects::Column::ObjectKey)
            .column(library_storage_objects::Column::StagingLeaseUntil)
            .into_model::<StorageObjectRecord>()
            .one(self.db.orm())
            .await?;
        Ok(row)
    }

    /// Live reference count for a storage object (library_files rows plus
    /// staged task inputs, plus task-item artifacts on the normal profile).
    /// Zero means a guarded unreferenced-object delete is allowed to remove it.
    pub async fn count_storage_object_references(&self, object_id: Uuid) -> Result<i64> {
        #[derive(sqlx::FromRow)]
        struct ReferenceCountRow {
            references: i64,
        }
        // The known pre-artifact recovery profile has no artifact relation, so
        // the two-referrer twin is the complete count there. Every other
        // database uses the artifact-aware statement and propagates its errors.
        let row = if self.db.pending_upgrade().is_some() {
            sqlx::query_file_as!(
                ReferenceCountRow,
                "src/sql/library_store/objects/count_storage_object_references_before_artifacts.sql",
                object_id
            )
            .fetch_one(self.db.pool())
            .await?
        } else {
            sqlx::query_file_as!(
                ReferenceCountRow,
                "src/sql/library_store/objects/count_storage_object_references.sql",
                object_id
            )
            .fetch_one(self.db.pool())
            .await?
        };
        Ok(row.references)
    }

    pub async fn get_storage_object_by_id_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object_by_id_on_connection.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn lock_storage_object(
        &self,
        connection: &mut sqlx::PgConnection,
        lock_key: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/objects/lock_storage_object.sql",
            lock_key
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    pub async fn get_storage_object_by_id_for_update(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
        before: DateTime<Utc>,
    ) -> Result<Option<StorageObjectRecord>> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file_as!(
                StorageObjectRecord,
                "src/sql/library_store/objects/get_storage_object_by_id_for_update_before_artifacts.sql",
                object_id,
                before
            )
            .fetch_optional(connection)
            .await?);
        }
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object_by_id_for_update.sql",
            object_id,
            before
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn upsert_storage_object(
        &self,
        id: Uuid,
        group_id: i64,
        sha256: &str,
        size_bytes: i64,
        storage_backend: &str,
        object_key: &str,
    ) -> Result<StorageObjectRecord> {
        let model = library_storage_objects::ActiveModel {
            id: Set(id),
            group_id: Set(group_id),
            sha256: Set(sha256.to_owned()),
            size_bytes: Set(size_bytes),
            storage_backend: Set(storage_backend.to_owned()),
            object_key: Set(object_key.to_owned()),
            staging_lease_until: sea_orm::ActiveValue::NotSet,
            created_at: sea_orm::ActiveValue::NotSet,
            updated_at: sea_orm::ActiveValue::NotSet,
        };
        let conflict = OnConflict::columns([
            library_storage_objects::Column::GroupId,
            library_storage_objects::Column::Sha256,
        ])
        .update_columns([
            library_storage_objects::Column::SizeBytes,
            library_storage_objects::Column::StorageBackend,
            library_storage_objects::Column::ObjectKey,
            library_storage_objects::Column::UpdatedAt,
        ])
        .to_owned();
        let row = library_storage_objects::Entity::insert(model)
            .on_conflict(conflict)
            .exec_with_returning(self.db.orm())
            .await?;
        Ok(StorageObjectRecord {
            id: row.id,
            group_id: row.group_id,
            sha256: row.sha256,
            size_bytes: row.size_bytes,
            storage_backend: row.storage_backend,
            object_key: row.object_key,
            staging_lease_until: row.staging_lease_until,
        })
    }

    pub async fn delete_unreferenced_storage_object(
        &self,
        id: Uuid,
    ) -> Result<Option<DeletedStorageObject>> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file_as!(
                DeletedStorageObject,
                "src/sql/library_store/objects/delete_unreferenced_storage_object_before_artifacts.sql",
                id
            )
            .fetch_optional(self.db.pool())
            .await?);
        }
        Ok(sqlx::query_file_as!(
            DeletedStorageObject,
            "src/sql/library_store/objects/delete_unreferenced_storage_object.sql",
            id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    /// Drop the staging lease of a committed artifact object.
    ///
    /// The statement itself requires a committed artifact reference and refuses
    /// an object that any task input still stages, so a shared input keeps its
    /// 24-hour protection. Returns whether the lease was cleared.
    pub async fn promote_task_item_artifact_object(&self, object_id: Uuid) -> Result<bool> {
        let mut connection = self.db.pool().acquire().await?;
        self.promote_task_item_artifact_object_in(&mut connection, object_id)
            .await
    }

    /// Drops the staging lease of a task artifact object on a borrowed
    /// connection.
    ///
    /// A task-owned promotion runs on the connection of its fenced write
    /// transaction, so the reference and the staging-lease drop commit
    /// together.
    pub async fn promote_task_item_artifact_object_in(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/objects/promote_task_item_artifact_object.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn clear_storage_object_staged(&self, object_id: Uuid, file_id: Uuid) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/objects/clear_staged_storage_object.sql",
            object_id,
            file_id
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn get_staged_storage_object_for_update(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file_as!(
                StorageObjectRecord,
                "src/sql/library_store/objects/get_staged_storage_object_for_update_before_artifacts.sql",
                object_id
            )
            .fetch_optional(connection)
            .await?);
        }
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_staged_storage_object_for_update.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn delete_released_staged_storage_object(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<bool> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file!(
                "src/sql/library_store/objects/delete_released_staged_storage_object_before_artifacts.sql",
                object_id
            )
            .fetch_optional(connection)
            .await?
            .is_some());
        }
        Ok(sqlx::query_file!(
            "src/sql/library_store/objects/delete_released_staged_storage_object.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn clear_expired_staging_with_file_reference(
        &self,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<u64> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file!(
                "src/sql/library_store/objects/clear_expired_staging_with_file_reference_before_artifacts.sql",
                before,
                limit
            )
            .execute(self.db.pool())
            .await?
            .rows_affected());
        }
        Ok(sqlx::query_file!(
            "src/sql/library_store/objects/clear_expired_staging_with_file_reference.sql",
            before,
            limit
        )
        .execute(self.db.pool())
        .await?
        .rows_affected())
    }

    pub async fn sweep_orphaned_storage_objects(
        &self,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<StorageObjectRecord>> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file_as!(
                StorageObjectRecord,
                "src/sql/library_store/objects/list_orphaned_storage_objects_before_artifacts.sql",
                before,
                limit
            )
            .fetch_all(self.db.pool())
            .await?);
        }
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/list_orphaned_storage_objects.sql",
            before,
            limit
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn delete_orphaned_storage_object_record_for_update(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
        before: DateTime<Utc>,
    ) -> Result<bool> {
        if self.db.pending_upgrade().is_some() {
            return Ok(sqlx::query_file!(
                "src/sql/library_store/objects/delete_orphaned_storage_object_record_for_update_before_artifacts.sql",
                object_id,
                before
            )
            .fetch_optional(connection)
            .await?
            .is_some());
        }
        Ok(sqlx::query_file!(
            "src/sql/library_store/objects/delete_orphaned_storage_object_record_for_update.sql",
            object_id,
            before
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    /// Record a successfully migrated legacy direct-path key for the later
    /// cleanup phase. Must run on the same connection/transaction as the
    /// reference update so both commit together. Re-runs are idempotent.
    pub async fn record_legacy_object_cleanup_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
        old_key: &str,
        old_storage_backend: &str,
        cleanup_eligible_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/objects/record_legacy_object_cleanup.sql",
            group_id,
            file_id,
            old_key,
            old_storage_backend,
            cleanup_eligible_at
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    /// Bounded, deterministic page of open legacy cleanup records whose grace
    /// period has elapsed. Pass `None` to start from the beginning; the id
    /// cursor keeps restarts and error retries safe.
    pub async fn list_eligible_legacy_cleanup(
        &self,
        after_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<LegacyCleanupRow>> {
        let mut query = library_legacy_object_cleanup::Entity::find()
            .select_only()
            .column(library_legacy_object_cleanup::Column::Id)
            .column(library_legacy_object_cleanup::Column::GroupId)
            .column(library_legacy_object_cleanup::Column::FileId)
            .column(library_legacy_object_cleanup::Column::OldKey)
            .column(library_legacy_object_cleanup::Column::OldStorageBackend)
            .filter(library_legacy_object_cleanup::Column::DeletedAt.is_null())
            .filter(cleanup_eligible_now());
        if let Some(after_id) = after_id {
            query = query.filter(library_legacy_object_cleanup::Column::Id.gt(after_id));
        }
        let rows = query
            .order_by_asc(library_legacy_object_cleanup::Column::Id)
            .limit(
                u64::try_from(limit)
                    .map_err(|_| anyhow::anyhow!("the page size must not be negative"))?,
            )
            .into_model::<LegacyCleanupRow>()
            .all(self.db.orm())
            .await?;
        Ok(rows)
    }

    /// Live check: how many library_files rows still point at this old key.
    /// Any nonzero result blocks physical deletion.
    pub async fn count_files_referencing_old_key(&self, old_key: &str) -> Result<i64> {
        let references = library_files::Entity::find()
            .select_only()
            .column(library_files::Column::Id)
            .filter(library_files::Column::StorageRelPath.eq(old_key))
            .into_tuple::<Uuid>()
            .count(self.db.orm())
            .await?;
        Ok(i64::try_from(references).unwrap_or(i64::MAX))
    }

    /// Conditionally close a cleanup record after its physical delete (or an
    /// already-missing observation). Returns `false` when the record was
    /// closed concurrently; the caller must not retry the write.
    pub async fn mark_legacy_cleanup_deleted(&self, id: i64) -> Result<bool> {
        let result = library_legacy_object_cleanup::Entity::update_many()
            .col_expr(
                library_legacy_object_cleanup::Column::DeletedAt,
                Expr::current_timestamp(),
            )
            .col_expr(
                library_legacy_object_cleanup::Column::DeleteError,
                Expr::val(None::<String>),
            )
            .filter(library_legacy_object_cleanup::Column::Id.eq(id))
            .filter(library_legacy_object_cleanup::Column::DeletedAt.is_null())
            .exec(self.db.orm())
            .await?;
        Ok(result.rows_affected > 0)
    }

    /// Persist a physical-delete failure on an open cleanup record so a later
    /// run retries it. Never marks the record deleted.
    pub async fn record_legacy_cleanup_error(&self, id: i64, error: &str) -> Result<()> {
        library_legacy_object_cleanup::Entity::update_many()
            .col_expr(
                library_legacy_object_cleanup::Column::DeleteError,
                Expr::val(error.to_owned()),
            )
            .filter(library_legacy_object_cleanup::Column::Id.eq(id))
            .filter(library_legacy_object_cleanup::Column::DeletedAt.is_null())
            .exec(self.db.orm())
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod native_query_tests {
    use super::*;
    use sea_orm::DbBackend;
    use sea_orm::QueryTrait;

    #[test]
    fn legacy_cleanup_scan_compares_against_the_server_clock() {
        let sql = library_legacy_object_cleanup::Entity::find()
            .select_only()
            .column(library_legacy_object_cleanup::Column::Id)
            .filter(library_legacy_object_cleanup::Column::DeletedAt.is_null())
            .filter(cleanup_eligible_now())
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains("\"cleanup_eligible_at\" <= CURRENT_TIMESTAMP"),
            "{sql}"
        );
        assert!(sql.contains("\"deleted_at\" IS NULL"), "{sql}");
    }

    #[test]
    fn storage_object_upsert_conflicts_on_the_group_content_pair() {
        let model = library_storage_objects::ActiveModel {
            id: Set(uuid::Uuid::nil()),
            group_id: Set(1),
            sha256: Set("a".to_owned()),
            size_bytes: Set(1),
            storage_backend: Set("local".to_owned()),
            object_key: Set("k".to_owned()),
            staging_lease_until: sea_orm::ActiveValue::NotSet,
            created_at: sea_orm::ActiveValue::NotSet,
            updated_at: sea_orm::ActiveValue::NotSet,
        };
        let conflict = OnConflict::columns([
            library_storage_objects::Column::GroupId,
            library_storage_objects::Column::Sha256,
        ])
        .update_columns([
            library_storage_objects::Column::SizeBytes,
            library_storage_objects::Column::StorageBackend,
            library_storage_objects::Column::ObjectKey,
            library_storage_objects::Column::UpdatedAt,
        ])
        .to_owned();
        let sql = library_storage_objects::Entity::insert(model)
            .on_conflict(conflict)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains("ON CONFLICT (\"group_id\", \"sha256\")"),
            "{sql}"
        );
        assert!(sql.contains("DO UPDATE SET"), "{sql}");
        assert!(
            sql.contains("\"updated_at\" = \"excluded\".\"updated_at\""),
            "{sql}"
        );
    }
}
