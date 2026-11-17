//! Native store access to the image/document occurrence links.
//!
//! The table is additive and owns no foreign key, so every read and write here
//! is explicitly group-scoped and every cleanup is an explicit call. All
//! expressions are built with the native ORM and bind their values as
//! parameters; the SQLx query cache is not involved.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use uuid::Uuid;

use super::LibraryStore;
use crate::db::entity::library_image_documents::{self, Column, Entity, Model};
use crate::domain_errors::DomainError;

/// The bounded number of occurrences one read returns.
pub const MAX_IMAGE_OCCURRENCES_PER_READ: u64 = 512;

/// The bounded number of documents one bulk link cleanup accepts.
pub const MAX_IMAGE_LINK_DOCUMENTS_PER_DELETE: usize = 1024;

/// One stored image/document occurrence link.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ImageDocumentLinkRecord {
    pub file_id: Uuid,
    pub document_id: i64,
    pub group_id: i64,
    pub role: String,
    pub ordinal: i32,
    pub caption: Option<String>,
    pub context_text: Option<String>,
    pub source_uri: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The identity and occurrence payload of one link write.
#[derive(Debug, Clone)]
pub struct NewImageDocumentLink<'a> {
    pub file_id: Uuid,
    pub document_id: i64,
    pub group_id: i64,
    pub role: &'a str,
    pub ordinal: i32,
    pub caption: Option<&'a str>,
    pub context_text: Option<&'a str>,
    pub source_uri: Option<&'a str>,
}

fn link_record(model: Model) -> ImageDocumentLinkRecord {
    ImageDocumentLinkRecord {
        file_id: model.file_id,
        document_id: model.document_id,
        group_id: model.group_id,
        role: model.role,
        ordinal: model.ordinal,
        caption: model.caption,
        context_text: model.context_text,
        source_uri: model.source_uri,
        created_at: model.created_at,
        updated_at: model.updated_at,
    }
}

impl LibraryStore {
    /// The link of one file at one occurrence slot, or `None`.
    pub async fn find_image_document_link(
        &self,
        group_id: i64,
        file_id: Uuid,
        document_id: i64,
        role: &str,
        ordinal: i32,
    ) -> Result<Option<ImageDocumentLinkRecord>> {
        let model = Entity::find()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::FileId.eq(file_id))
            .filter(Column::DocumentId.eq(document_id))
            .filter(Column::Role.eq(role))
            .filter(Column::Ordinal.eq(ordinal))
            .one(self.db.orm())
            .await?;
        Ok(model.map(link_record))
    }

    /// The image of one document occurrence slot, whichever file stores it.
    pub async fn find_image_occurrence(
        &self,
        group_id: i64,
        document_id: i64,
        role: &str,
        ordinal: i32,
    ) -> Result<Option<ImageDocumentLinkRecord>> {
        let model = Entity::find()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::DocumentId.eq(document_id))
            .filter(Column::Role.eq(role))
            .filter(Column::Ordinal.eq(ordinal))
            .one(self.db.orm())
            .await?;
        Ok(model.map(link_record))
    }

    /// Write one link, converging an identical repeat of the same occurrence.
    ///
    /// The write is group-scoped and refuses a slot that another file already
    /// owns, so an occurrence never silently changes its image and no link is
    /// ever attached across groups.
    pub async fn upsert_image_document_link(
        &self,
        link: &NewImageDocumentLink<'_>,
    ) -> Result<ImageDocumentLinkRecord> {
        if let Some(existing) = self
            .find_image_occurrence(link.group_id, link.document_id, link.role, link.ordinal)
            .await?
            && existing.file_id != link.file_id
        {
            return Err(DomainError::conflict(
                "the document occurrence already belongs to another image",
            )
            .into());
        }
        let now = Utc::now();
        let active = library_image_documents::ActiveModel {
            file_id: Set(link.file_id),
            document_id: Set(link.document_id),
            group_id: Set(link.group_id),
            role: Set(link.role.to_string()),
            ordinal: Set(link.ordinal),
            caption: Set(link.caption.map(str::to_string)),
            context_text: Set(link.context_text.map(str::to_string)),
            source_uri: Set(link.source_uri.map(str::to_string)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let conflict = OnConflict::columns([
            Column::FileId,
            Column::DocumentId,
            Column::Role,
            Column::Ordinal,
        ])
        .update_columns([
            Column::GroupId,
            Column::Caption,
            Column::ContextText,
            Column::SourceUri,
            Column::UpdatedAt,
        ])
        .to_owned();
        Entity::insert(active)
            .on_conflict(conflict)
            .exec(self.db.orm())
            .await
            .map_err(unique_violation_as_conflict)?;
        self.find_image_document_link(
            link.group_id,
            link.file_id,
            link.document_id,
            link.role,
            link.ordinal,
        )
        .await?
        .ok_or_else(|| DomainError::not_found("image document link not found").into())
    }

    /// The bounded occurrences of one file inside one group.
    pub async fn list_image_document_links_for_file(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<Vec<ImageDocumentLinkRecord>> {
        let models = Entity::find()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::FileId.eq(file_id))
            .order_by_asc(Column::DocumentId)
            .order_by_asc(Column::Role)
            .order_by_asc(Column::Ordinal)
            .limit(MAX_IMAGE_OCCURRENCES_PER_READ)
            .all(self.db.orm())
            .await?;
        Ok(models.into_iter().map(link_record).collect())
    }

    /// The bounded occurrences of one document inside one group.
    pub async fn list_image_document_links_for_document(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<Vec<ImageDocumentLinkRecord>> {
        let models = Entity::find()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::DocumentId.eq(document_id))
            .order_by_asc(Column::Role)
            .order_by_asc(Column::Ordinal)
            .limit(MAX_IMAGE_OCCURRENCES_PER_READ)
            .all(self.db.orm())
            .await?;
        Ok(models.into_iter().map(link_record).collect())
    }

    /// Delete one scoped file/document occurrence link.
    pub async fn delete_image_document_link(
        &self,
        group_id: i64,
        file_id: Uuid,
        document_id: i64,
        role: &str,
        ordinal: i32,
    ) -> Result<bool> {
        let deleted = Entity::delete_many()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::FileId.eq(file_id))
            .filter(Column::DocumentId.eq(document_id))
            .filter(Column::Role.eq(role))
            .filter(Column::Ordinal.eq(ordinal))
            .exec(self.db.orm())
            .await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Delete every scoped occurrence link of one file.
    pub async fn delete_image_document_links_for_file(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<u64> {
        let deleted = Entity::delete_many()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::FileId.eq(file_id))
            .exec(self.db.orm())
            .await?;
        Ok(deleted.rows_affected)
    }

    /// Delete every scoped occurrence link of one document.
    pub async fn delete_image_document_links_for_document(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<u64> {
        let deleted = Entity::delete_many()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::DocumentId.eq(document_id))
            .exec(self.db.orm())
            .await?;
        Ok(deleted.rows_affected)
    }

    /// Delete the scoped occurrence links of a bounded document set.
    pub async fn delete_image_document_links_for_documents(
        &self,
        group_id: i64,
        document_ids: &[i64],
    ) -> Result<u64> {
        if document_ids.is_empty() {
            return Ok(0);
        }
        if document_ids.len() > MAX_IMAGE_LINK_DOCUMENTS_PER_DELETE {
            return Err(DomainError::invalid_argument(format!(
                "at most {MAX_IMAGE_LINK_DOCUMENTS_PER_DELETE} documents are deleted at once"
            ))
            .into());
        }
        let deleted = Entity::delete_many()
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::DocumentId.is_in(document_ids.to_vec()))
            .exec(self.db.orm())
            .await?;
        Ok(deleted.rows_affected)
    }

    /// The stored filename and media type of one file, read inside the locked
    /// link transaction.
    pub(crate) async fn image_file_media_in_group_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<Option<(String, String)>> {
        Ok(sqlx::query_as(include_str!(
            "../sql/db/library_image_documents/file_exists_in_group.sql"
        ))
        .bind(file_id)
        .bind(group_id)
        .fetch_optional(connection)
        .await?)
    }

    /// The parent document id of one group key, read inside the locked link
    /// transaction.
    pub(crate) async fn image_document_id_by_key_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        source_key: &str,
        external_id: &str,
    ) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar(include_str!(
            "../sql/db/library_image_documents/document_id_by_key.sql"
        ))
        .bind(group_id)
        .bind(source_key)
        .bind(external_id)
        .fetch_optional(connection)
        .await?)
    }

    /// The file id that owns one occurrence slot, read inside the locked link
    /// transaction.
    pub(crate) async fn image_occurrence_file_id_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        document_id: i64,
        role: &str,
        ordinal: i32,
    ) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(include_str!(
            "../sql/db/library_image_documents/find_occurrence.sql"
        ))
        .bind(group_id)
        .bind(document_id)
        .bind(role)
        .bind(ordinal)
        .fetch_optional(connection)
        .await?)
    }

    /// Write one occurrence link on a caller's locked transaction.
    pub(crate) async fn upsert_image_document_link_on(
        &self,
        connection: &mut sqlx::PgConnection,
        link: &NewImageDocumentLink<'_>,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/library_image_documents/upsert_link.sql"
        ))
        .bind(link.file_id)
        .bind(link.document_id)
        .bind(link.group_id)
        .bind(link.role)
        .bind(link.ordinal)
        .bind(link.caption)
        .bind(link.context_text)
        .bind(link.source_uri)
        .execute(connection)
        .await?;
        Ok(())
    }

    /// Delete one occurrence link on a caller's locked transaction.
    pub(crate) async fn delete_image_document_link_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
        document_id: i64,
        role: &str,
        ordinal: i32,
    ) -> Result<bool> {
        let deleted = sqlx::query(include_str!(
            "../sql/db/library_image_documents/delete_link.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .bind(document_id)
        .bind(role)
        .bind(ordinal)
        .execute(connection)
        .await?
        .rows_affected();
        Ok(deleted > 0)
    }

    /// Delete every link of one file on a caller's transaction.
    pub(crate) async fn delete_image_document_links_for_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<u64> {
        let result = sqlx::query(include_str!(
            "../sql/db/library_image_documents/delete_for_file.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected())
    }

    /// Delete every link of the documents that one file owns, on a caller's
    /// transaction. A file delete removes those parent documents, so an image
    /// from another file must stop pointing at them.
    pub(crate) async fn delete_image_document_links_for_file_documents_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<u64> {
        let result = sqlx::query(include_str!(
            "../sql/db/library_image_documents/delete_for_file_documents.sql"
        ))
        .bind(file_id)
        .bind(group_id)
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected())
    }

    /// Delete every link of one document on a caller's transaction.
    pub(crate) async fn delete_image_document_links_for_document_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        document_id: i64,
    ) -> Result<u64> {
        let result = sqlx::query(include_str!(
            "../sql/db/library_image_documents/delete_for_document.sql"
        ))
        .bind(group_id)
        .bind(document_id)
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected())
    }

    /// Delete every link of the documents in one folder subtree on a caller's
    /// transaction, before the documents themselves are removed.
    pub(crate) async fn delete_image_document_links_for_folder_subtree_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        folder_id: Uuid,
    ) -> Result<u64> {
        let result = sqlx::query(include_str!(
            "../sql/db/library_image_documents/delete_for_folder_subtree.sql"
        ))
        .bind(folder_id)
        .bind(group_id)
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected())
    }
}

/// Map the occurrence unique index to the same conflict the service reports.
fn unique_violation_as_conflict(error: sea_orm::DbErr) -> anyhow::Error {
    if matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        return DomainError::conflict("the document occurrence already belongs to another image")
            .into();
    }
    error.into()
}
