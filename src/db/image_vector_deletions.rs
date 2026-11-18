//! Durable image-vector origins and deletion targets.
//!
//! Every image write registers the exact physical point it is about to produce
//! before the Qdrant call. A full image file or group delete turns those
//! origins into exact deletion targets, so cleanup never infers a location
//! from the current settings or media type. A bounded cleanup pass claims one
//! target at a time and stamps the attempt before the network call, so an
//! unreachable origin moves to the back of the queue instead of pinning the
//! head of every later pass. A document delete never records a target: an
//! image shared by several parents stays searchable with its remaining links.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use super::Database;
use super::entity::image_vector_deletions::{Column, Entity};

/// One pending image-vector deletion target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageVectorDeletion {
    pub id: i64,
    pub group_id: i64,
    pub file_id: Uuid,
    pub attempt_id: Uuid,
    pub point_id: Uuid,
    pub content_sha256: String,
    pub runtime_fingerprint: String,
    pub endpoint: String,
    pub collection_name: String,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
}

/// The exact physical point one image write registers before writing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImageVectorOrigin<'a> {
    pub attempt_id: Uuid,
    pub group_id: i64,
    pub file_id: Uuid,
    pub content_sha256: &'a str,
    pub runtime_fingerprint: &'a str,
    pub endpoint: &'a str,
    pub collection_name: &'a str,
    pub point_id: Uuid,
}

/// Whether a replacement changed the content revision that image vectors
/// describe. A same-content metadata replay must keep the active image.
pub(crate) fn image_content_revision_changed(current_sha256: &str, next_sha256: &str) -> bool {
    current_sha256 != next_sha256
}

type UncertainOriginRow = (
    Uuid,
    i64,
    Uuid,
    String,
    String,
    String,
    String,
    Uuid,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// One durable unacknowledged image write attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageVectorOrigin {
    pub attempt_id: Uuid,
    pub group_id: i64,
    pub file_id: Uuid,
    pub content_sha256: String,
    pub runtime_fingerprint: String,
    pub endpoint: String,
    pub collection_name: String,
    pub point_id: Uuid,
    pub attempt_started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One image search candidate proven by its exact active origin and current
/// file revision.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ActiveImageSearchCandidate {
    pub point_id: Uuid,
    pub file_id: Uuid,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub sha256: String,
}

impl Database {
    /// Acquire the shared image-vector mutation scopes of one transaction.
    ///
    /// Every image link write, image vector write, and image-affecting delete
    /// uses this same deterministic order: the group scope first, then each
    /// file scope in sorted order, then each document scope in sorted order.
    pub async fn lock_image_vector_scopes_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_ids: &[Uuid],
        document_ids: &[i64],
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_origins/lock_group.sql"
        ))
        .bind(group_id)
        .execute(&mut *connection)
        .await?;
        let mut files = file_ids.to_vec();
        files.sort_unstable();
        files.dedup();
        for file_id in files {
            sqlx::query(include_str!("../sql/db/image_vector_origins/lock_file.sql"))
                .bind(file_id)
                .execute(&mut *connection)
                .await?;
        }
        let mut documents = document_ids.to_vec();
        documents.sort_unstable();
        documents.dedup();
        for document_id in documents {
            sqlx::query(include_str!(
                "../sql/db/image_vector_origins/lock_document.sql"
            ))
            .bind(document_id)
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
    }

    /// Register the exact origin of one image write before the Qdrant call.
    pub async fn register_image_vector_origin_in(
        &self,
        connection: &mut PgConnection,
        origin: &NewImageVectorOrigin<'_>,
    ) -> Result<()> {
        sqlx::query(include_str!("../sql/db/image_vector_origins/upsert.sql"))
            .bind(origin.attempt_id)
            .bind(origin.group_id)
            .bind(origin.file_id)
            .bind(origin.content_sha256)
            .bind(origin.runtime_fingerprint)
            .bind(origin.endpoint)
            .bind(origin.collection_name)
            .bind(origin.point_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    /// Whether the registered origin still owns the exact point.
    pub async fn image_vector_origin_is_current_in(
        &self,
        connection: &mut PgConnection,
        attempt_id: Uuid,
        file_id: Uuid,
        point_id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query(include_str!(
            "../sql/db/image_vector_origins/is_current.sql"
        ))
        .bind(attempt_id)
        .bind(file_id)
        .bind(point_id)
        .fetch_optional(&mut *connection)
        .await?
        .is_some())
    }

    /// Whether another attempt already published this file after the current
    /// attempt registered.
    pub async fn image_vector_newer_active_origin_exists_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query(include_str!(
            "../sql/db/image_vector_origins/newer_active_exists.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .bind(attempt_id)
        .fetch_optional(&mut *connection)
        .await?
        .is_some())
    }

    /// Acknowledge the exact point after Qdrant confirms the write.
    ///
    /// A missing origin row means a delete took ownership of the point, so the
    /// caller must not report success.
    pub async fn mark_image_vector_origin_active_in(
        &self,
        connection: &mut PgConnection,
        attempt_id: Uuid,
        file_id: Uuid,
        point_id: Uuid,
    ) -> Result<bool> {
        let updated = sqlx::query(include_str!(
            "../sql/db/image_vector_origins/mark_active.sql"
        ))
        .bind(attempt_id)
        .bind(file_id)
        .bind(point_id)
        .execute(&mut *connection)
        .await?
        .rows_affected();
        Ok(updated > 0)
    }

    /// Turn every recorded origin of one file into an exact deletion target.
    pub async fn schedule_image_vector_deletions_for_file_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<()> {
        sqlx::query(include_str!("../sql/db/image_vector_deletions/record.sql"))
            .bind(group_id)
            .bind(file_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    /// Turn every recorded origin of one group into an exact deletion target.
    pub async fn schedule_image_vector_deletions_for_group_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_deletions/record_group.sql"
        ))
        .bind(group_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Turn the obsolete origins of one file into exact deletion targets.
    pub async fn schedule_obsolete_image_vector_deletions_for_file_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_id: Uuid,
        winning_attempt_id: Uuid,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_deletions/record_obsolete.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .bind(winning_attempt_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Drop the origin rows of one file after they became deletion targets.
    pub async fn delete_image_vector_origins_for_file_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_origins/delete_file_origins.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// The bounded set of writing attempts older than the publish window.
    pub async fn claim_uncertain_image_vector_origins(
        &self,
        updated_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ImageVectorOrigin>> {
        let rows: Vec<UncertainOriginRow> = sqlx::query_as(include_str!(
            "../sql/db/image_vector_origins/select_uncertain.sql"
        ))
        .bind(updated_before)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    attempt_id,
                    group_id,
                    file_id,
                    content_sha256,
                    runtime_fingerprint,
                    endpoint,
                    collection_name,
                    point_id,
                    attempt_started_at,
                    updated_at,
                )| ImageVectorOrigin {
                    attempt_id,
                    group_id,
                    file_id,
                    content_sha256,
                    runtime_fingerprint,
                    endpoint,
                    collection_name,
                    point_id,
                    attempt_started_at,
                    updated_at,
                },
            )
            .collect())
    }

    /// Whether the attempt is still an unacknowledged writing origin older
    /// than the bounded publish window.
    pub async fn image_vector_origin_is_uncertain_in(
        &self,
        connection: &mut PgConnection,
        attempt_id: Uuid,
        updated_before: DateTime<Utc>,
    ) -> Result<bool> {
        Ok(sqlx::query(include_str!(
            "../sql/db/image_vector_origins/is_uncertain.sql"
        ))
        .bind(attempt_id)
        .bind(updated_before)
        .fetch_optional(&mut *connection)
        .await?
        .is_some())
    }

    /// Turn one abandoned origin into an exact deletion target.
    pub async fn schedule_image_vector_deletion_for_origin_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_id: Uuid,
        attempt_id: Uuid,
        point_id: Uuid,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_deletions/record_one.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .bind(attempt_id)
        .bind(point_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Drop one abandoned origin after it became a deletion target.
    pub async fn delete_image_vector_origin_in(
        &self,
        connection: &mut PgConnection,
        attempt_id: Uuid,
        file_id: Uuid,
        point_id: Uuid,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_origins/delete_one.sql"
        ))
        .bind(attempt_id)
        .bind(file_id)
        .bind(point_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Drop the origin rows of one group after they became deletion targets.
    pub async fn delete_image_vector_origins_for_group_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_origins/delete_group_origins.sql"
        ))
        .bind(group_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Drop the obsolete origin rows of one file after they became targets.
    pub async fn delete_obsolete_image_vector_origins_in(
        &self,
        connection: &mut PgConnection,
        group_id: i64,
        file_id: Uuid,
        winning_attempt_id: Uuid,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_origins/delete_obsolete_origins.sql"
        ))
        .bind(group_id)
        .bind(file_id)
        .bind(winning_attempt_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// The bounded set of pending targets, oldest attempt first.
    pub async fn claim_pending_image_vector_deletions(
        &self,
        limit: i64,
    ) -> Result<Vec<ImageVectorDeletion>> {
        let rows = Entity::find()
            .filter(Column::PurgedAt.is_null())
            .order_by_asc(Column::UpdatedAt)
            .order_by_asc(Column::Id)
            .limit(limit.max(1) as u64)
            .all(&self.orm)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| ImageVectorDeletion {
                id: row.id,
                group_id: row.group_id,
                file_id: row.file_id,
                attempt_id: row.attempt_id,
                point_id: row.point_id,
                content_sha256: row.content_sha256,
                runtime_fingerprint: row.runtime_fingerprint,
                endpoint: row.endpoint,
                collection_name: row.collection_name,
                attempts: row.attempts,
                created_at: row.created_at.into(),
            })
            .collect())
    }

    /// Stamp one attempt before its network call.
    pub async fn mark_image_vector_deletion_attempt(
        &self,
        id: i64,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_deletions/mark_attempt.sql"
        ))
        .bind(id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Hydrate image search candidates against their exact active origin and
    /// current file revision.
    pub async fn active_image_search_candidates(
        &self,
        group_id: i64,
        endpoint: &str,
        collection_name: &str,
        runtime_fingerprint: &str,
        point_ids: &[Uuid],
    ) -> Result<Vec<ActiveImageSearchCandidate>> {
        if point_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as(include_str!(
            "../sql/db/image_vector_origins/select_active_for_points.sql"
        ))
        .bind(group_id)
        .bind(endpoint)
        .bind(collection_name)
        .bind(runtime_fingerprint)
        .bind(point_ids)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Stamp the confirmed physical removal.
    pub async fn mark_image_vector_deletion_purged(&self, id: i64) -> Result<()> {
        sqlx::query(include_str!(
            "../sql/db/image_vector_deletions/mark_purged.sql"
        ))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::image_content_revision_changed;

    #[test]
    fn a_same_content_replay_does_not_invalidate_the_active_image() {
        assert!(!image_content_revision_changed("sha-a", "sha-a"));
    }

    #[test]
    fn a_changed_content_revision_retires_the_previous_image_origin() {
        assert!(image_content_revision_changed("sha-a", "sha-b"));
    }
}
