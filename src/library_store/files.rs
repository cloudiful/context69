use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{
    Alias, ColumnRef, Expr, Query as QuerySelectBuilder, ReturningClause, SelectStatement, TableRef,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, UpdateMany, Value,
};
use sqlx::FromRow;
use uuid::Uuid;

/// Legacy direct-path row selected by the migration tool: the file still
/// references its old UUID-path object directly (no storage object yet).
#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
pub struct LegacyDirectPathFileRow {
    pub id: Uuid,
    pub group_id: i64,
    pub filename: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub created_at: DateTime<Utc>,
}

/// Current storage linkage of a library_files row, used to classify why a
/// conditional legacy reference update did not land.
#[derive(Debug, FromRow, sea_orm::FromQueryResult)]
pub struct LegacyFileStorageStateRow {
    pub storage_object_id: Option<Uuid>,
    pub storage_rel_path: String,
}

/// Legacy direct-path row eligible for the startup missing-source cleanup:
/// still pointing at the legacy key, in a terminal ingest state, and old
/// enough that the active storage key has had a chance to settle.
#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
pub struct MissingLegacySourceFileRow {
    pub id: Uuid,
    pub group_id: i64,
    pub filename: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub ingest_status: String,
    pub created_at: DateTime<Utc>,
}

/// The library file row a delete locks.
///
/// The columns are the identity the delete needs: the group of the tombstone
/// audit value, the storage reference of the source cleanup intent and the
/// release state that keeps a deliberately released source out of the cleanup.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct DeleteFileRow {
    pub id: Uuid,
    pub group_id: i64,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    pub source_released_at: Option<DateTime<Utc>>,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct ReplacementFileSnapshot {
    pub id: Uuid,
    pub group_id: i64,
    pub folder_id: Option<Uuid>,
    pub visibility: String,
    pub external_id: Option<String>,
    pub source_uri: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub metadata_json: serde_json::Value,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    pub source_released_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, sea_orm::FromQueryResult)]
struct FileTranslationDirectiveRow {
    translation_override: bool,
    translation_source_locale: Option<String>,
    translation_target_locales: Vec<String>,
}

use super::mappers::file_from_row;
use super::{
    FileRow, LibraryFileRecord, LibraryIngestStatus, LibraryStore, NewLibraryFile, TaskWriteDenied,
    TaskWriteFence, UpdateLibraryFileContent, group_projection, update_with_projection,
};
use crate::db::entity::groups;
use crate::db::entity::library_files;

/// The bounded page size of a native list.
///
/// A negative page size is refused the same way Postgres refuses a negative
/// `LIMIT`.
fn positive_limit(limit: i64) -> Result<u64> {
    u64::try_from(limit).map_err(|_| anyhow::anyhow!("the page size must not be negative"))
}

/// The group id of the library file row.
fn library_files_group_id() -> ColumnRef {
    ColumnRef::from((Alias::new("library_files"), library_files::Column::GroupId))
}

/// The library file rows with the derived group key and group path.
fn file_select() -> sea_orm::Select<library_files::Entity> {
    library_files::Entity::find()
        .select_only()
        .column(library_files::Column::GroupId)
        .expr_as(
            group_projection(groups::Column::GroupKey, library_files_group_id()),
            "group_key",
        )
        .expr_as(
            group_projection(groups::Column::FullPath, library_files_group_id()),
            "group_path",
        )
        .column(library_files::Column::Visibility)
        .column(library_files::Column::Id)
        .column(library_files::Column::FolderId)
        .column(library_files::Column::ExternalId)
        .column(library_files::Column::SourceUri)
        .column(library_files::Column::PublishedAt)
        .column(library_files::Column::MetadataJson)
        .column(library_files::Column::Filename)
        .column(library_files::Column::MediaType)
        .column(library_files::Column::SizeBytes)
        .column(library_files::Column::Sha256)
        .column(library_files::Column::StorageRelPath)
        .column(library_files::Column::IngestStatus)
        .column(library_files::Column::ErrorMessage)
        .column(library_files::Column::CreatedAt)
        .column(library_files::Column::UpdatedAt)
        .column(library_files::Column::IngestedAt)
}

/// The projection of the file rows an update returned.
///
/// The columns are the projection fields of the plain file read, in the same
/// order, and the group key and the group path are derived from the owning
/// group exactly like the plain read derives them.
fn file_update_projection(cte_name: &str) -> SelectStatement {
    let cte = Alias::new(cte_name);
    let column = |name: &str| Expr::col(ColumnRef::from((cte.clone(), Alias::new(name))));
    let owner_group_id = ColumnRef::from((cte.clone(), Alias::new("group_id")));
    QuerySelectBuilder::select()
        .expr_as(column("group_id"), "group_id")
        .expr_as(
            group_projection(groups::Column::GroupKey, owner_group_id.clone()),
            "group_key",
        )
        .expr_as(
            group_projection(groups::Column::FullPath, owner_group_id),
            "group_path",
        )
        .expr_as(column("visibility"), "visibility")
        .expr_as(column("id"), "id")
        .expr_as(column("folder_id"), "folder_id")
        .expr_as(column("external_id"), "external_id")
        .expr_as(column("source_uri"), "source_uri")
        .expr_as(column("published_at"), "published_at")
        .expr_as(column("metadata_json"), "metadata_json")
        .expr_as(column("filename"), "filename")
        .expr_as(column("media_type"), "media_type")
        .expr_as(column("size_bytes"), "size_bytes")
        .expr_as(column("sha256"), "sha256")
        .expr_as(column("storage_rel_path"), "storage_rel_path")
        .expr_as(column("ingest_status"), "ingest_status")
        .expr_as(column("error_message"), "error_message")
        .expr_as(column("created_at"), "created_at")
        .expr_as(column("updated_at"), "updated_at")
        .expr_as(column("ingested_at"), "ingested_at")
        .from(TableRef::from(cte))
        .to_owned()
}

impl LibraryStore {
    /// Read and lock one file row on a caller transaction.
    ///
    /// The metadata publisher takes this row first so it serializes with the
    /// replacement switch, which locks the same row before its documents.
    pub async fn lock_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/lock_file_row.sql",
            file_id
        )
        .fetch_optional(connection)
        .await?;
        row.map(file_from_row).transpose()
    }

    pub async fn lock_replacement_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
    ) -> Result<Option<ReplacementFileSnapshot>> {
        Ok(sqlx::query_file_as!(
            ReplacementFileSnapshot,
            "src/sql/library_store/files/lock_replacement_snapshot.sql",
            file_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn switch_replacement_file_in(
        &self,
        write: &mut super::TaskWriteTx<'_>,
        baseline: &ReplacementFileSnapshot,
        staged_object: Option<Uuid>,
        attributes: Option<&crate::services::library::ReplacementFileAttributes>,
    ) -> Result<bool> {
        let current = self
            .lock_replacement_file_on(write.connection(), baseline.id)
            .await?;
        if current.as_ref() != Some(baseline) {
            return Ok(false);
        }
        let item_id = write.fence().item_id();
        let (folder_id, external_id, filename, media_type, source_uri, published_at, metadata_json) =
            match attributes {
                Some(attributes) => (
                    attributes.folder_id,
                    attributes.external_id.as_deref(),
                    Some(attributes.filename.as_str()),
                    Some(attributes.media_type.as_str()),
                    attributes.source_uri.as_deref(),
                    attributes.published_at,
                    Some(&attributes.metadata_json),
                ),
                None => (None, None, None, None, None, None, None),
            };
        let result = sqlx::query_file_scalar!(
            "src/sql/library_store/files/switch_replacement_success.sql",
            baseline.id,
            baseline.group_id,
            baseline.updated_at,
            staged_object,
            item_id,
            folder_id,
            external_id,
            filename,
            media_type,
            source_uri,
            published_at,
            metadata_json,
            attributes.is_some()
        )
        .fetch_optional(write.connection())
        .await;
        match result {
            Ok(id) => Ok(id.is_some()),
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23505") => {
                Err(crate::domain_errors::DomainError::conflict(
                    "a replaced file conflicts with another file in the group",
                )
                .into())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn set_file_translation_directive(
        &self,
        file_id: Uuid,
        directive: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<()> {
        library_files::Entity::update_many()
            .col_expr(
                library_files::Column::TranslationOverride,
                Expr::val(directive.is_some()),
            )
            .col_expr(
                library_files::Column::TranslationSourceLocale,
                Expr::val(directive.and_then(|value| value.source_locale.clone())),
            )
            .col_expr(
                library_files::Column::TranslationTargetLocales,
                Expr::val(
                    directive
                        .map(|value| value.target_locales.clone())
                        .unwrap_or_default(),
                ),
            )
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::Id.eq(file_id))
            .exec(self.db.orm())
            .await?;
        Ok(())
    }

    /// The translation directive of one file on a caller transaction.
    ///
    /// The atomic first insert publishes the directive of the created file on
    /// the transaction that creates it, so the directive commits with the file
    /// and never describes a row that does not exist.
    pub async fn set_file_translation_directive_in(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        directive: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/files/set_translation_directive.sql",
            file_id,
            directive.is_some(),
            directive.and_then(|value| value.source_locale.as_deref()),
            directive
                .map(|value| value.target_locales.as_slice())
                .unwrap_or_default()
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    pub async fn file_translation_directive(
        &self,
        file_id: Uuid,
    ) -> Result<Option<crate::contracts::TranslationDirective>> {
        let row = library_files::Entity::find_by_id(file_id)
            .select_only()
            .column(library_files::Column::TranslationOverride)
            .column(library_files::Column::TranslationSourceLocale)
            .column(library_files::Column::TranslationTargetLocales)
            .into_model::<FileTranslationDirectiveRow>()
            .one(self.db.orm())
            .await?;
        Ok(row.and_then(|row| {
            row.translation_override
                .then_some(crate::contracts::TranslationDirective {
                    source_locale: row.translation_source_locale,
                    target_locales: row.translation_target_locales,
                })
        }))
    }

    pub async fn update_business_metadata(
        &self,
        group_id: i64,
        file_id: Uuid,
        metadata: &crate::contracts::CanonicalUploadMetadata,
    ) -> Result<Option<LibraryFileRecord>> {
        let metadata_json = crate::contracts::metadata_object_to_value(&metadata.metadata_json);
        let update = library_files::Entity::update_many()
            .col_expr(
                library_files::Column::ExternalId,
                Expr::val(metadata.external_id.clone()),
            )
            .col_expr(
                library_files::Column::SourceUri,
                Expr::val(metadata.source_uri.clone()),
            )
            .col_expr(
                library_files::Column::PublishedAt,
                Expr::val(metadata.published_at),
            )
            .col_expr(
                library_files::Column::MetadataJson,
                Expr::val(metadata_json),
            )
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::GroupId.eq(group_id))
            .filter(library_files::Column::Id.eq(file_id));
        self.updated_file(update).await
    }

    /// The business metadata rewrite of one file on a caller transaction.
    ///
    /// The metadata publisher writes the file row and every linked document on
    /// the same connection, so a later document failure rolls the file back too.
    pub async fn update_business_metadata_in(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
        metadata: &crate::contracts::CanonicalUploadMetadata,
    ) -> Result<Option<LibraryFileRecord>> {
        let metadata_json = crate::contracts::metadata_object_to_value(&metadata.metadata_json);
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_business_metadata.sql",
            group_id,
            file_id,
            metadata.external_id,
            metadata.source_uri,
            metadata.published_at,
            metadata_json
        )
        .fetch_optional(connection)
        .await?;
        row.map(file_from_row).transpose()
    }
    pub async fn list_files(&self) -> Result<Vec<LibraryFileRecord>> {
        let rows = file_select()
            .order_by_asc(library_files::Column::Filename)
            .order_by_asc(library_files::Column::Id)
            .into_model::<FileRow>()
            .all(self.db.orm())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    /// Bounded page of legacy direct-path rows ordered by the
    /// `(created_at, id)` cursor. Pass `None` for both cursor values to start
    /// from the beginning.
    pub async fn list_legacy_direct_path_files(
        &self,
        after_created_at: Option<DateTime<Utc>>,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<LegacyDirectPathFileRow>> {
        let mut query = library_files::Entity::find()
            .select_only()
            .column(library_files::Column::Id)
            .column(library_files::Column::GroupId)
            .column(library_files::Column::Filename)
            .column(library_files::Column::SizeBytes)
            .column(library_files::Column::Sha256)
            .column(library_files::Column::StorageRelPath)
            .column(library_files::Column::CreatedAt)
            .filter(library_files::Column::StorageObjectId.is_null());
        if let Some(after_created_at) = after_created_at {
            query = query.filter(Expr::cust_with_values(
                "(created_at, id) > ($1, $2)",
                [Value::from(after_created_at), Value::from(after_id)],
            ));
        }
        let rows = query
            .order_by_asc(library_files::Column::CreatedAt)
            .order_by_asc(library_files::Column::Id)
            .limit(positive_limit(limit)?)
            .into_model::<LegacyDirectPathFileRow>()
            .all(self.db.orm())
            .await?;
        Ok(rows)
    }

    /// Bounded page of legacy direct-path rows whose source object is a
    /// candidate for startup missing-source cleanup. Filters out rows that
    /// were concurrently linked (`storage_object_id IS NOT NULL`), ingested
    /// again (non-terminal `ingest_status`), or are too recent to safely
    /// verify (younger than `grace_hours`).
    pub async fn list_missing_legacy_source_files(
        &self,
        grace_hours: i64,
        after_created_at: Option<DateTime<Utc>>,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<MissingLegacySourceFileRow>> {
        let mut query = library_files::Entity::find()
            .select_only()
            .column(library_files::Column::Id)
            .column(library_files::Column::GroupId)
            .column(library_files::Column::Filename)
            .column(library_files::Column::SizeBytes)
            .column(library_files::Column::Sha256)
            .column(library_files::Column::StorageRelPath)
            .column(library_files::Column::IngestStatus)
            .column(library_files::Column::CreatedAt)
            .filter(library_files::Column::StorageObjectId.is_null())
            .filter(library_files::Column::SourceReleasedAt.is_null())
            .filter(library_files::Column::IngestStatus.is_in(["succeeded", "failed"]))
            .filter(Expr::cust_with_values(
                "created_at < now() - ($1::bigint * INTERVAL '1 hour')",
                [Value::from(grace_hours)],
            ));
        if let Some(after_created_at) = after_created_at {
            query = query.filter(Expr::cust_with_values(
                "(created_at, id) > ($1, $2)",
                [Value::from(after_created_at), Value::from(after_id)],
            ));
        }
        let rows = query
            .order_by_asc(library_files::Column::CreatedAt)
            .order_by_asc(library_files::Column::Id)
            .limit(positive_limit(limit)?)
            .into_model::<MissingLegacySourceFileRow>()
            .all(self.db.orm())
            .await?;
        Ok(rows)
    }

    /// Lightweight existence check for any legacy direct-path rows. Used by
    /// startup to gate the old-key cleanup phase: legacy old-key deletion
    /// only runs when this returns `false` so a partially-completed
    /// migration cannot strand old objects that the missing-source cleanup
    /// is still about to act on.
    pub async fn has_legacy_direct_path_files(&self) -> Result<bool> {
        let rows = self
            .list_legacy_direct_path_files(None, None, 1_i64)
            .await?;
        Ok(!rows.is_empty())
    }

    /// Conditionally link a legacy direct-path row to a content-addressed
    /// storage object. Returns `false` when the row no longer matches the old
    /// key or was linked concurrently; the caller must then treat its own
    /// object as unreferenced.
    pub async fn link_legacy_file_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        expected_old_key: &str,
        object_id: Uuid,
        object_key: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/files/link_legacy_file_storage_object.sql",
            file_id,
            object_id,
            object_key,
            expected_old_key
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn get_legacy_file_storage_state(
        &self,
        file_id: Uuid,
    ) -> Result<Option<LegacyFileStorageStateRow>> {
        let row = library_files::Entity::find_by_id(file_id)
            .select_only()
            .column(library_files::Column::StorageObjectId)
            .column(library_files::Column::StorageRelPath)
            .into_model::<LegacyFileStorageStateRow>()
            .one(self.db.orm())
            .await?;
        Ok(row)
    }

    pub async fn list_files_by_ids(&self, file_ids: &[Uuid]) -> Result<Vec<LibraryFileRecord>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = file_select()
            .filter(Expr::cust_with_values(
                "id = ANY($1)",
                [Value::from(file_ids.to_vec())],
            ))
            .order_by_asc(library_files::Column::Filename)
            .order_by_asc(library_files::Column::Id)
            .into_model::<FileRow>()
            .all(self.db.orm())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn list_files_in_project(&self, project_id: i64) -> Result<Vec<LibraryFileRecord>> {
        let rows = file_select()
            .filter(library_files::Column::GroupId.eq(project_id))
            .order_by_asc(library_files::Column::Filename)
            .order_by_asc(library_files::Column::Id)
            .into_model::<FileRow>()
            .all(self.db.orm())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn list_filenames_in_project_folder(
        &self,
        project_id: i64,
        folder_id: Option<Uuid>,
        exclude_file_id: Option<Uuid>,
    ) -> Result<Vec<String>> {
        let mut condition = Condition::all().add(library_files::Column::GroupId.eq(project_id));
        condition = match folder_id {
            Some(folder_id) => condition.add(library_files::Column::FolderId.eq(folder_id)),
            None => condition.add(library_files::Column::FolderId.is_null()),
        };
        if let Some(exclude_file_id) = exclude_file_id {
            condition = condition.add(library_files::Column::Id.ne(exclude_file_id));
        }
        Ok(library_files::Entity::find()
            .select_only()
            .column(library_files::Column::Filename)
            .filter(condition)
            .order_by_asc(library_files::Column::Filename)
            .order_by_asc(library_files::Column::Id)
            .into_tuple::<String>()
            .all(self.db.orm())
            .await?)
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<Option<LibraryFileRecord>> {
        let row = file_select()
            .filter(library_files::Column::Id.eq(file_id))
            .into_model::<FileRow>()
            .one(self.db.orm())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = file_select()
            .filter(library_files::Column::GroupId.eq(project_id))
            .filter(library_files::Column::Id.eq(file_id))
            .into_model::<FileRow>()
            .one(self.db.orm())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_by_external_id_in_project(
        &self,
        project_id: i64,
        external_id: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = file_select()
            .filter(library_files::Column::GroupId.eq(project_id))
            .filter(library_files::Column::ExternalId.eq(external_id))
            .into_model::<FileRow>()
            .one(self.db.orm())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_by_sha_in_project(
        &self,
        project_id: i64,
        sha256: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = file_select()
            .filter(library_files::Column::GroupId.eq(project_id))
            .filter(library_files::Column::Sha256.eq(sha256))
            .order_by_asc(library_files::Column::CreatedAt)
            .order_by_asc(library_files::Column::Id)
            .limit(1)
            .into_model::<FileRow>()
            .one(self.db.orm())
            .await?;

        row.map(file_from_row).transpose()
    }

    /// The candidate of one library identity on a caller transaction.
    ///
    /// The atomic first insert re-probes the identity inside the transaction
    /// that creates the file, so the read must run on that transaction's own
    /// connection and see the rows the first writer committed.
    pub async fn get_file_by_external_id_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
        project_id: i64,
        external_id: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/get_file_by_external_id_in_project.sql",
            project_id,
            external_id
        )
        .fetch_optional(connection)
        .await?;

        row.map(file_from_row).transpose()
    }

    /// The same-content candidate of one library identity on a caller
    /// transaction.
    pub async fn get_file_by_sha_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
        project_id: i64,
        sha256: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/get_file_by_sha_in_project.sql",
            project_id,
            sha256
        )
        .fetch_optional(connection)
        .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn create_file(&self, file: &NewLibraryFile) -> Result<LibraryFileRecord> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/create_file.sql",
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            file.storage_object_id,
            file.delete_source_after_processing
        )
        .fetch_one(self.db.pool())
        .await?;

        file_from_row(row)
    }

    pub async fn create_file_in_project(
        &self,
        project_id: i64,
        file: &NewLibraryFile,
    ) -> Result<LibraryFileRecord> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/create_file_in_project.sql",
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            project_id,
            file.storage_object_id,
            file.delete_source_after_processing
        )
        .fetch_one(self.db.pool())
        .await?;

        file_from_row(row)
    }

    /// The create statement on a caller transaction.
    ///
    /// The atomic first insert writes the file row on the connection of the
    /// transaction that binds the task item and claims the durable run, so a
    /// created file is never observable without its item and run binding.
    pub async fn create_file_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
        project_id: i64,
        file: &NewLibraryFile,
    ) -> Result<LibraryFileRecord> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/create_file_in_project.sql",
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            project_id,
            file.storage_object_id,
            file.delete_source_after_processing
        )
        .fetch_one(connection)
        .await?;

        file_from_row(row)
    }

    /// The create statement inside the fenced write of a running task item.
    ///
    /// The library row is written on the connection of the guarded transaction,
    /// so the file and the grant that authorized it commit together. A refused
    /// grant returns `None` and nothing is written.
    pub async fn create_file_in_project_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file: &NewLibraryFile,
    ) -> Result<Option<LibraryFileRecord>> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/create_file_in_project.sql",
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            project_id,
            file.storage_object_id,
            file.delete_source_after_processing
        )
        .fetch_optional(write.connection())
        .await?;
        let Some(row) = row else {
            write.rollback().await;
            return Ok(None);
        };
        let record = file_from_row(row)?;
        if !write.commit().await? {
            return Ok(None);
        }
        Ok(Some(record))
    }

    /// The content update inside the fenced write of a running task item.
    pub async fn update_file_content_in_project_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file_id: Uuid,
        update: &UpdateLibraryFileContent,
    ) -> Result<Option<LibraryFileRecord>> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_file_content_in_project.sql",
            project_id,
            file_id,
            update.folder_id,
            update.external_id,
            update.filename,
            update.media_type,
            update.size_bytes,
            update.sha256,
            update.storage_rel_path,
            update.storage_object_id
        )
        .fetch_optional(write.connection())
        .await?;
        let Some(row) = row else {
            write.rollback().await;
            return Ok(None);
        };
        let record = file_from_row(row)?;
        if !write.commit().await? {
            return Ok(None);
        }
        Ok(Some(record))
    }

    pub async fn update_file_content_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        update: &UpdateLibraryFileContent,
    ) -> Result<Option<LibraryFileRecord>> {
        let native = library_files::Entity::update_many()
            .col_expr(library_files::Column::FolderId, Expr::val(update.folder_id))
            .col_expr(
                library_files::Column::ExternalId,
                Expr::val(update.external_id.clone()),
            )
            .col_expr(
                library_files::Column::Filename,
                Expr::val(update.filename.clone()),
            )
            .col_expr(
                library_files::Column::MediaType,
                Expr::val(update.media_type.clone()),
            )
            .col_expr(
                library_files::Column::SizeBytes,
                Expr::val(update.size_bytes),
            )
            .col_expr(
                library_files::Column::Sha256,
                Expr::val(update.sha256.clone()),
            )
            .col_expr(
                library_files::Column::StorageRelPath,
                Expr::val(update.storage_rel_path.clone()),
            )
            .col_expr(
                library_files::Column::StorageObjectId,
                Expr::val(update.storage_object_id),
            )
            .col_expr(
                library_files::Column::SourceReleasedAt,
                Expr::val(None::<DateTime<Utc>>),
            )
            .col_expr(library_files::Column::IngestStatus, Expr::val("failed"))
            .col_expr(
                library_files::Column::ErrorMessage,
                Expr::val("awaiting first ingest"),
            )
            .col_expr(
                library_files::Column::IngestedAt,
                Expr::val(None::<DateTime<Utc>>),
            )
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::GroupId.eq(project_id))
            .filter(library_files::Column::Id.eq(file_id));
        self.updated_file(native).await
    }

    pub async fn restore_file_snapshot_in_project(
        &self,
        file: &crate::domain::LibraryFileRecord,
        storage_object_id: Option<Uuid>,
        source_released_at: Option<DateTime<Utc>>,
    ) -> Result<Option<LibraryFileRecord>> {
        let native = library_files::Entity::update_many()
            .col_expr(library_files::Column::FolderId, Expr::val(file.folder_id))
            .col_expr(
                library_files::Column::ExternalId,
                Expr::val(file.external_id.clone()),
            )
            .col_expr(
                library_files::Column::Filename,
                Expr::val(file.filename.clone()),
            )
            .col_expr(
                library_files::Column::MediaType,
                Expr::val(file.media_type.clone()),
            )
            .col_expr(library_files::Column::SizeBytes, Expr::val(file.size_bytes))
            .col_expr(
                library_files::Column::Sha256,
                Expr::val(file.sha256.clone()),
            )
            .col_expr(
                library_files::Column::StorageRelPath,
                Expr::val(file.storage_rel_path.clone()),
            )
            .col_expr(
                library_files::Column::StorageObjectId,
                Expr::val(storage_object_id),
            )
            .col_expr(
                library_files::Column::SourceReleasedAt,
                Expr::val(source_released_at),
            )
            .col_expr(
                library_files::Column::IngestStatus,
                Expr::val(file.ingest_status.as_str()),
            )
            .col_expr(
                library_files::Column::ErrorMessage,
                Expr::val(file.error_message.clone()),
            )
            .col_expr(
                library_files::Column::IngestedAt,
                Expr::val(file.ingested_at),
            )
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::GroupId.eq(file.group_id))
            .filter(library_files::Column::Id.eq(file.id));
        self.updated_file(native).await
    }

    /// The compensation of one failed task upload, in one guarded write.
    ///
    /// The previous snapshot of the file, its business metadata, its
    /// translation directive and the search generation counter are restored on
    /// the connection of the guarded transaction of the *original* captured
    /// grant, and the grant is checked again before the commit. `false` means
    /// the grant is gone: nothing is restored, and the caller must not fall back
    /// to the pooled helpers, because the row may already belong to a newer
    /// owner.
    pub async fn restore_file_snapshot_for_task_fenced(
        &self,
        fence: TaskWriteFence,
        file: &crate::domain::LibraryFileRecord,
        storage_object_id: Option<Uuid>,
        source_released_at: Option<DateTime<Utc>>,
        metadata: &crate::contracts::CanonicalUploadMetadata,
        translation: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        let metadata_json = crate::contracts::metadata_object_to_value(&metadata.metadata_json);
        let restored = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/restore_file_snapshot_in_project.sql",
            file.group_id,
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            storage_object_id,
            file.ingest_status.as_str(),
            file.error_message,
            file.ingested_at,
            source_released_at
        )
        .fetch_optional(write.connection())
        .await?;
        if restored.is_none() {
            write.rollback().await;
            return Ok(false);
        }
        let updated = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_business_metadata.sql",
            file.group_id,
            file.id,
            metadata.external_id,
            metadata.source_uri,
            metadata.published_at,
            metadata_json
        )
        .fetch_optional(write.connection())
        .await?;
        if updated.is_none() {
            write.rollback().await;
            return Ok(false);
        }
        sqlx::query_file!(
            "src/sql/library_store/files/set_translation_directive.sql",
            file.id,
            translation.is_some(),
            translation.and_then(|value| value.source_locale.as_deref()),
            translation
                .map(|value| value.target_locales.as_slice())
                .unwrap_or_default()
        )
        .execute(write.connection())
        .await?;
        // The counter is advanced on the guarded connection with the existing
        // checked statement, so the restored snapshot and the cache
        // invalidation it implies commit together.
        sqlx::query_file_scalar!("src/sql/db/search_cache/bump_search_generation.sql")
            .fetch_one(write.connection())
            .await?;
        write.commit().await
    }

    /// Removes the file record of one failed task upload under its grant.
    ///
    /// `false` means the grant is gone and nothing was removed.
    pub async fn rollback_file_record_for_task_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        self.delete_file_in_project_fenced(fence, project_id, file_id)
            .await
    }

    pub async fn move_file(
        &self,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        self.move_file_native(None, file_id, target_folder_id).await
    }

    /// The file move on a caller transaction.
    ///
    /// The UPDATE locks the file row as the first statement, so the metadata
    /// refresh that follows in the same transaction is serialized with the
    /// replacement switch and commits or rolls back with the move.
    pub async fn move_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/move_file.sql",
            file_id,
            target_folder_id
        )
        .fetch_optional(connection)
        .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn move_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        self.move_file_native(Some(project_id), file_id, target_folder_id)
            .await
    }

    pub async fn move_file_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
        project_id: i64,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/move_file_in_project.sql",
            project_id,
            file_id,
            target_folder_id
        )
        .fetch_optional(connection)
        .await?;

        row.map(file_from_row).transpose()
    }

    /// The native file move.
    ///
    /// The move and the projection of the moved row share one transaction, so
    /// the row lock of the move is held until the returned row is read.
    async fn move_file_native(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let mut update = library_files::Entity::update_many()
            .col_expr(library_files::Column::FolderId, Expr::val(target_folder_id))
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::Id.eq(file_id));
        if let Some(project_id) = project_id {
            update = update.filter(library_files::Column::GroupId.eq(project_id));
        }
        self.updated_file(update).await
    }

    /// The status projection of one file, written inside the guarded write of
    /// a running task item.
    ///
    /// A worker that lost its segment projects nothing: the statement runs on
    /// the connection of the guarded transaction, and the grant is rechecked
    /// before the commit.
    pub async fn update_file_status_fenced(
        &self,
        fence: TaskWriteFence,
        file_id: Uuid,
        status: LibraryIngestStatus,
        error_message: Option<&str>,
        mark_ingested_now: bool,
    ) -> Result<Option<LibraryFileRecord>> {
        let ingested_at = if mark_ingested_now {
            Some(Utc::now())
        } else {
            None
        };
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_file_status.sql",
            file_id,
            status.as_str(),
            error_message,
            ingested_at
        )
        .fetch_optional(write.connection())
        .await?;
        let Some(row) = row else {
            write.rollback().await;
            return Ok(None);
        };
        let record = file_from_row(row)?;
        if !write.commit().await? {
            return Ok(None);
        }
        Ok(Some(record))
    }

    pub async fn update_file_status(
        &self,
        file_id: Uuid,
        status: LibraryIngestStatus,
        error_message: Option<&str>,
        mark_ingested_now: bool,
    ) -> Result<Option<LibraryFileRecord>> {
        let ingested_at = if mark_ingested_now {
            Some(Utc::now())
        } else {
            None
        };
        let native = library_files::Entity::update_many()
            .col_expr(
                library_files::Column::IngestStatus,
                Expr::val(status.as_str()),
            )
            .col_expr(
                library_files::Column::ErrorMessage,
                Expr::val(error_message.map(|message| message.to_owned())),
            )
            .col_expr(library_files::Column::IngestedAt, Expr::val(ingested_at))
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::Id.eq(file_id))
            .filter(library_files::Column::IngestStatus.eq("failed"));
        self.updated_file(native).await
    }

    /// The native file update and its projection.
    ///
    /// The update and the projection of the updated row are one statement, so
    /// the returned row is the row of the update statement's own snapshot. A
    /// guarded update that matched no row returns no row at all.
    async fn updated_file(
        &self,
        update: UpdateMany<library_files::Entity>,
    ) -> Result<Option<LibraryFileRecord>> {
        let mut statement = update.into_query();
        statement.returning(ReturningClause::All);
        let rows = self
            .db
            .orm()
            .query_all(&update_with_projection(
                statement,
                "updated_library_files",
                file_update_projection("updated_library_files"),
            ))
            .await?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(file_from_row(FileRow::from_query_result(&row, "")?)?))
    }

    pub async fn delete_file_record(&self, file_id: Uuid) -> Result<bool> {
        let result = library_files::Entity::delete_many()
            .filter(library_files::Column::Id.eq(file_id))
            .exec(self.db.orm())
            .await?;
        Ok(result.rows_affected > 0)
    }

    /// Read and lock one library file row for a delete.
    ///
    /// The delete path locks the registry epoch first and this row second, so
    /// the epoch is always the first lock of the transaction. `project_id`
    /// scopes the lock to one group: a delete of another project never locks
    /// (and never deletes) a file it does not own.
    pub async fn lock_file_for_delete_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        project_id: Option<i64>,
    ) -> Result<Option<DeleteFileRow>> {
        let row = match project_id {
            Some(project_id) => {
                sqlx::query_file_as!(
                    DeleteFileRow,
                    "src/sql/library_store/files/lock_file_for_delete_in_project.sql",
                    file_id,
                    project_id
                )
                .fetch_optional(connection)
                .await?
            }
            None => {
                sqlx::query_file_as!(
                    DeleteFileRow,
                    "src/sql/library_store/files/lock_file_for_delete.sql",
                    file_id
                )
                .fetch_optional(connection)
                .await?
            }
        };
        Ok(row)
    }

    /// Delete one library file row on a borrowed connection.
    ///
    /// The delete backstop trigger of the row records the entity tombstone of
    /// the file, so a delete that reaches this statement always leaves the
    /// exclusion behind. A supported caller has already recorded the same
    /// tombstone, which makes the trigger a no-op instead of a second bump.
    pub async fn delete_file_record_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/library_store/files/delete_file_record.sql",
            file_id
        )
        .execute(connection)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete one library file row of one project on a borrowed connection.
    pub async fn delete_file_record_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/library_store/files/delete_file_record_in_project.sql",
            project_id,
            file_id
        )
        .execute(connection)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Delete one file row under the captured grant of a running task item.
    ///
    /// The documents of the file and the file row are removed in one guarded
    /// transaction, so a worker that lost its segment cannot delete anything and
    /// a crash between the two deletes cannot leave a file without documents.
    pub async fn delete_file_in_project_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        self.delete_documents_for_files_in(write.connection(), &[file_id])
            .await?;
        let deleted = sqlx::query_file!(
            "src/sql/library_store/files/delete_file_record_in_project.sql",
            project_id,
            file_id
        )
        .execute(write.connection())
        .await?
        .rows_affected()
            > 0;
        if !deleted {
            write.rollback().await;
            return Ok(false);
        }
        write.commit().await
    }

    pub async fn delete_file_record_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        let result = library_files::Entity::delete_many()
            .filter(library_files::Column::GroupId.eq(project_id))
            .filter(library_files::Column::Id.eq(file_id))
            .exec(self.db.orm())
            .await?;
        Ok(result.rows_affected > 0)
    }
}

#[cfg(test)]
mod native_query_tests {
    use super::*;
    use sea_orm::DbBackend;
    use sea_orm::QueryTrait;
    use sea_orm::StatementBuilder;

    fn file_id() -> Uuid {
        Uuid::nil()
    }

    #[test]
    fn native_reads_project_the_derived_group_columns() {
        let sql = file_select()
            .filter(library_files::Column::Id.eq(file_id()))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(
                "(SELECT \"groups\".\"group_key\" FROM \"context69\".\"groups\" WHERE \"groups\".\"id\" = \"library_files\".\"group_id\") AS \"group_key\""
            ),
            "{sql}"
        );
        assert!(
            sql.contains(
                "(SELECT \"groups\".\"full_path\" FROM \"context69\".\"groups\" WHERE \"groups\".\"id\" = \"library_files\".\"group_id\") AS \"group_path\""
            ),
            "{sql}"
        );
        assert!(
            sql.contains("FROM \"context69\".\"library_files\""),
            "{sql}"
        );
    }

    #[test]
    fn native_update_projects_the_updated_row_in_one_statement() {
        let mut statement = library_files::Entity::update_many()
            .col_expr(
                library_files::Column::FolderId,
                Expr::val(Option::<Uuid>::None),
            )
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::Id.eq(file_id()))
            .into_query();
        statement.returning(ReturningClause::All);
        let statement = update_with_projection(
            statement,
            "updated_library_files",
            file_update_projection("updated_library_files"),
        );
        let sql = StatementBuilder::build(&statement, &DbBackend::Postgres).sql;

        assert!(
            sql.contains(
                "WITH \"updated_library_files\" AS (UPDATE \"context69\".\"library_files\" SET"
            ),
            "{sql}"
        );
        assert!(sql.contains("RETURNING *)"), "{sql}");
        assert!(
            sql.contains(
                "(SELECT \"groups\".\"full_path\" FROM \"context69\".\"groups\" WHERE \"groups\".\"id\" = \"updated_library_files\".\"group_id\") AS \"group_path\""
            ),
            "{sql}"
        );
        assert!(sql.contains("FROM \"updated_library_files\""), "{sql}");
    }

    #[test]
    fn native_list_by_ids_binds_the_ids_as_one_array() {
        let sql = file_select()
            .filter(Expr::cust_with_values(
                "id = ANY($1)",
                [Value::from(vec![file_id()])],
            ))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains("id = ANY("), "{sql}");
        assert!(sql.contains(&file_id().to_string()), "{sql}");
    }

    #[test]
    fn native_status_update_keeps_the_failed_status_guard() {
        let sql = library_files::Entity::update_many()
            .col_expr(library_files::Column::IngestStatus, Expr::val("succeeded"))
            .col_expr(library_files::Column::UpdatedAt, Expr::current_timestamp())
            .filter(library_files::Column::Id.eq(file_id()))
            .filter(library_files::Column::IngestStatus.eq("failed"))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains("UPDATE \"context69\".\"library_files\""),
            "{sql}"
        );
        assert!(sql.contains("\"ingest_status\" = 'failed'"), "{sql}");
        assert!(sql.contains("\"ingest_status\" = 'succeeded'"), "{sql}");
    }

    #[test]
    fn native_folder_names_reject_a_deleted_file_only_when_it_is_known() {
        let included = library_files::Entity::find()
            .select_only()
            .column(library_files::Column::Filename)
            .filter(library_files::Column::GroupId.eq(1))
            .filter(library_files::Column::FolderId.is_null())
            .filter(library_files::Column::Id.ne(file_id()))
            .build(DbBackend::Postgres)
            .to_string();
        assert!(included.contains("\"folder_id\" IS NULL"), "{included}");
        assert!(included.contains("\"id\" <> "), "{included}");
    }
}
