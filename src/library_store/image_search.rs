//! Bounded read hydration for scoped image search and document image lists.

use anyhow::Result;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use super::{ImageDocumentLinkRecord, LibraryStore};
use crate::db::entity::{library_files, library_image_documents};
use crate::domain::AccessScope;

/// The bounded number of parent occurrences hydrated for one search hit.
pub const IMAGE_PARENT_HYDRATION_LIMIT: usize = 32;

/// One current image file header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryImageFileRecord {
    pub file_id: Uuid,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub sha256: String,
}

/// One authorized parent document header.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ImageParentDocumentRecord {
    pub document_id: i64,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub source_uri: String,
}

impl LibraryStore {
    /// The bounded occurrence links of a set of images.
    ///
    /// At most `per_file_limit + 1` rows are returned per file, so a caller can
    /// report `has_more` without an additional count.
    pub async fn image_links_for_files_bounded(
        &self,
        group_id: i64,
        file_ids: &[Uuid],
        per_file_limit: usize,
    ) -> Result<Vec<ImageDocumentLinkRecord>> {
        if file_ids.is_empty() || per_file_limit == 0 {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as(include_str!(
            "../sql/db/library_image_documents/list_file_links_bounded.sql"
        ))
        .bind(group_id)
        .bind(file_ids)
        .bind((per_file_limit + 1) as i64)
        .fetch_all(self.db.pool())
        .await?)
    }

    /// The current files of a set of image ids inside one group.
    pub async fn images_by_ids_in_group(
        &self,
        group_id: i64,
        file_ids: &[Uuid],
    ) -> Result<Vec<LibraryImageFileRecord>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let models = library_files::Entity::find()
            .filter(library_files::Column::GroupId.eq(group_id))
            .filter(library_files::Column::Id.is_in(file_ids.to_vec()))
            .all(self.db.orm())
            .await?;
        Ok(models
            .into_iter()
            .map(|model| LibraryImageFileRecord {
                file_id: model.id,
                external_id: model.external_id,
                filename: model.filename,
                media_type: model.media_type,
                sha256: model.sha256,
            })
            .collect())
    }

    /// The bounded occurrence links of one document.
    ///
    /// One extra row is requested so the caller can report `has_more`.
    pub async fn image_links_for_document_bounded(
        &self,
        group_id: i64,
        document_id: i64,
        limit: usize,
    ) -> Result<Vec<ImageDocumentLinkRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let models = library_image_documents::Entity::find()
            .filter(library_image_documents::Column::GroupId.eq(group_id))
            .filter(library_image_documents::Column::DocumentId.eq(document_id))
            .order_by_asc(library_image_documents::Column::Role)
            .order_by_asc(library_image_documents::Column::Ordinal)
            .order_by_asc(library_image_documents::Column::FileId)
            .limit((limit + 1) as u64)
            .all(self.db.orm())
            .await?;
        Ok(models.into_iter().map(link_record).collect())
    }

    /// Hydrate the authorized parent headers of one bounded document set.
    pub async fn authorized_image_parent_documents(
        &self,
        group_id: i64,
        document_ids: &[i64],
        scope: &AccessScope,
    ) -> Result<Vec<ImageParentDocumentRecord>> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as(include_str!(
            "../sql/db/library_image_documents/authorized_parent_documents.sql"
        ))
        .bind(group_id)
        .bind(document_ids)
        .bind(&scope.private_group_ids)
        .bind(scope.group_path.as_deref())
        .fetch_all(self.db.pool())
        .await?)
    }
}

fn link_record(model: library_image_documents::Model) -> ImageDocumentLinkRecord {
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
