use std::collections::HashMap;

use anyhow::{Context, Result};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use super::{
    Database, DocumentRow, ExistingDocumentRow, KeywordSearchHitRow, ReadDocumentChunkBatchRow,
    ReadDocumentChunkRow, ReindexChunkRow, SearchHitRow, TranslationChunkBatchRow,
    TranslationStatusBatchRow, TranslationVersionBatchRow, UpsertedDocument, is_library_file,
    keyword_terms, library_file_id, library_path, library_section_label,
    search_hit_from_keyword_row, translation_status,
};
use crate::chunk_payload::{RowPayload, from_row};
use crate::contracts::{
    DocumentChunkResponse, DocumentResponse, SearchHit, SearchRequest, TranslationStatus,
    Visibility,
};
use crate::domain::{AccessScope, ChunkPayload, DocumentChunk};
use crate::normalize::is_meaningful_text;

/// How many reserved document ids may be tombstoned in a row before the
/// reservation is refused.
///
/// The bound keeps the reservation bounded: a run of tombstoned candidates
/// ends the attempt with an error instead of consuming the sequence without
/// an end.
const RESERVED_DOCUMENT_ID_TOMBSTONE_SKIP_LIMIT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ReplacementDocumentSnapshot {
    pub id: i64,
    pub group_id: i64,
    pub source_key: String,
    pub external_id: String,
    pub record_hash: String,
}

/// The canonical business source of one document for a metadata publication.
///
/// `body_text` is the body of the current `document_versions` row, joined by the
/// document's current record hash. It is `None` when that version row is
/// missing, and the caller must then fail closed.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DocumentBusinessSource {
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at_source: chrono::DateTime<chrono::Utc>,
    pub metadata_json: serde_json::Value,
    pub record_hash: String,
    pub body_text: Option<String>,
}

impl Database {
    /// The ready-metadata definitions of one group and source on a caller
    /// connection.
    ///
    /// Metadata publication validates and indexes on the same connection as its
    /// writes, so no pooled read observes a different transaction.
    pub(crate) async fn list_metadata_indexes_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        source_key: &str,
    ) -> Result<Vec<super::metadata_indexes::StoredMetadataIndex>> {
        super::metadata_indexes::list_metadata_indexes_in(connection, group_id, source_key).await
    }

    /// The canonical business source of one document, locked for the caller
    /// transaction and joined with its current version body.
    pub async fn get_document_business_source_on(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
    ) -> Result<Option<DocumentBusinessSource>> {
        Ok(sqlx::query_file_as!(
            DocumentBusinessSource,
            "src/sql/db/documents/get_document_business_source.sql",
            document_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn lock_replacement_documents_on(
        &self,
        connection: &mut sqlx::PgConnection,
        ids: &[i64],
    ) -> Result<Vec<ReplacementDocumentSnapshot>> {
        Ok(sqlx::query_file_as!(
            ReplacementDocumentSnapshot,
            "src/sql/db/documents/lock_replacement_documents.sql",
            ids
        )
        .fetch_all(connection)
        .await?)
    }

    /// The point ids of one affected document that a replacement retires.
    ///
    /// `record_hash` is the desired NEW source hash of the replacement: the
    /// document's own chunk rows are always part of the set, and a stored
    /// translation chunk joins it when its version was translated from a
    /// different source hash.
    pub async fn replacement_point_ids_on(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
        record_hash: &str,
        invalidate_translation: bool,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/documents/read_replacement_point_ids.sql",
            document_id,
            record_hash,
            invalidate_translation
        )
        .fetch_all(connection)
        .await?)
    }

    /// The translation point ids of one document that the desired NEW source
    /// hash invalidates.
    ///
    /// The document's own chunks stay live: a metadata publication rewrites
    /// their payload in place. A translation chunk that a registry entry
    /// already excludes is not reported again, so a repeated invalidation of
    /// the same translation never fails the bounded admission.
    pub async fn stale_translation_point_ids_on(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
        record_hash: &str,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/documents/read_stale_translation_point_ids.sql",
            document_id,
            record_hash
        )
        .fetch_all(connection)
        .await?)
    }

    pub async fn find_document_id_by_key(
        &self,
        group_id: i64,
        source_key: &str,
        external_id: &str,
    ) -> Result<Option<i64>> {
        use super::entity::documents::{Column, Entity};
        Ok(Entity::find()
            .select_only()
            .column(Column::Id)
            .filter(Column::GroupId.eq(group_id))
            .filter(Column::SourceKey.eq(source_key))
            .filter(Column::ExternalId.eq(external_id))
            .into_tuple::<i64>()
            .one(&self.orm)
            .await?)
    }

    /// The one library file that links this document, or `None`.
    ///
    /// A document with no section mapping and a document shared by more than
    /// one file both return `None`, so a publication that needs a single file
    /// scope never guesses one.
    pub async fn find_unique_library_file_for_document(
        &self,
        document_id: i64,
    ) -> Result<Option<Uuid>> {
        use super::entity::library_file_documents::{Column, Entity};
        let mut file_ids = Entity::find()
            .select_only()
            .column(Column::FileId)
            .distinct()
            .filter(Column::DocumentId.eq(document_id))
            .limit(2)
            .into_tuple::<Uuid>()
            .all(&self.orm)
            .await?;
        if file_ids.len() != 1 {
            return Ok(None);
        }
        Ok(file_ids.pop())
    }

    pub async fn list_document_ids_by_keys(
        &self,
        group_id: i64,
        source_keys: &[String],
        external_ids: &[String],
    ) -> Result<Vec<Option<i64>>> {
        if source_keys.is_empty() {
            return Ok(Vec::new());
        }
        if source_keys.len() != external_ids.len() {
            return Err(anyhow::anyhow!(
                "document key arrays must have the same length"
            ));
        }

        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Func, JoinType, Order, Query},
        };
        let requested = Query::select()
            .expr_as(
                Func::cust(Alias::new("unnest")).arg(source_keys.to_vec()),
                Alias::new("source_key"),
            )
            .expr_as(
                Func::cust(Alias::new("unnest")).arg(external_ids.to_vec()),
                Alias::new("external_id"),
            )
            .expr_as(
                Func::cust(Alias::new("generate_subscripts"))
                    .arg(source_keys.to_vec())
                    .arg(1),
                Alias::new("ordinal"),
            )
            .to_owned();
        let requested_name = Alias::new("requested");
        let documents = Alias::new("documents");
        let query = Query::select()
            .column((requested_name.clone(), Alias::new("ordinal")))
            .expr_as(
                Expr::col((documents.clone(), Alias::new("id"))),
                Alias::new("document_id"),
            )
            .from_subquery(requested, requested_name.clone())
            .join(
                JoinType::LeftJoin,
                (Alias::new("context69"), documents.clone()),
                Expr::col((documents.clone(), Alias::new("group_id")))
                    .eq(group_id)
                    .and(
                        Expr::col((documents.clone(), Alias::new("source_key")))
                            .equals((requested_name.clone(), Alias::new("source_key"))),
                    )
                    .and(
                        Expr::col((documents.clone(), Alias::new("external_id")))
                            .equals((requested_name.clone(), Alias::new("external_id"))),
                    ),
            )
            .order_by((requested_name, Alias::new("ordinal")), Order::Asc)
            .to_owned();
        let rows = self.orm.query_all(&query).await?;

        let mut result = vec![None; source_keys.len()];
        for row in rows {
            let ordinal: i32 = row.try_get("", "ordinal")?;
            let position = usize::try_from(ordinal.saturating_sub(1))
                .context("document key ordinal is out of range")?;
            if let Some(slot) = result.get_mut(position) {
                *slot = row.try_get("", "document_id")?;
            }
        }
        Ok(result)
    }

    pub async fn get_documents_localized(
        &self,
        document_ids: &[i64],
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<HashMap<i64, DocumentResponse>> {
        if document_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let ids = document_ids.to_vec();
        let documents = sqlx::query_file_as!(
            DocumentRow,
            "src/sql/db/documents/get_documents_by_ids.sql",
            &ids,
            &scope.private_group_ids,
            scope.group_path.as_deref()
        )
        .fetch_all(&self.pool)
        .await?;
        let chunks = sqlx::query_file_as!(
            ReadDocumentChunkBatchRow,
            "src/sql/db/documents/get_read_document_chunks_by_ids.sql",
            &ids
        )
        .fetch_all(&self.pool)
        .await?;

        let mut chunks_by_document = HashMap::<i64, Vec<DocumentChunkResponse>>::new();
        for chunk in chunks {
            if is_meaningful_text(&chunk.chunk_text) {
                chunks_by_document
                    .entry(chunk.document_id)
                    .or_default()
                    .push(DocumentChunkResponse {
                        chunk_id: chunk.id,
                        chunk_index: chunk.chunk_index,
                        text: chunk.chunk_text,
                        record_hash: chunk.record_hash,
                    });
            }
        }

        let mut result = HashMap::with_capacity(documents.len());
        for document in documents {
            let document_id = document.id;
            let chunks = chunks_by_document.remove(&document_id).unwrap_or_default();
            result.insert(
                document_id,
                document_response_from_parts(document, chunks, None, None, false),
            );
        }

        let Some(locale) = locale else {
            for document in result.values_mut() {
                document.content_locale = Some("original".to_string());
            }
            return Ok(result);
        };

        let versions = sqlx::query_file_as!(
            TranslationVersionBatchRow,
            "src/sql/db/translations/get_versions_by_document_ids.sql",
            &ids,
            locale
        )
        .fetch_all(&self.pool)
        .await?;
        let version_ids = versions
            .iter()
            .map(|version| version.id)
            .collect::<Vec<_>>();
        let translation_chunks = if version_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_file_as!(
                TranslationChunkBatchRow,
                "src/sql/db/translations/get_chunks_by_version_ids.sql",
                &version_ids
            )
            .fetch_all(&self.pool)
            .await?
        };
        let statuses = sqlx::query_file_as!(
            TranslationStatusBatchRow,
            "src/sql/db/translations/get_status_by_document_ids.sql",
            &ids,
            locale
        )
        .fetch_all(&self.pool)
        .await?;

        let versions_by_document = versions
            .into_iter()
            .map(|version| (version.document_id, version))
            .collect::<HashMap<_, _>>();
        let mut chunks_by_version = HashMap::<Uuid, Vec<TranslationChunkBatchRow>>::new();
        for chunk in translation_chunks {
            chunks_by_version
                .entry(chunk.translation_id)
                .or_default()
                .push(chunk);
        }
        let statuses_by_document = statuses
            .into_iter()
            .map(|status| (status.document_id, status))
            .collect::<HashMap<_, _>>();

        for (document_id, document) in &mut result {
            if let Some(version) = versions_by_document.get(document_id) {
                document.title = version.translated_title.clone();
                document.summary = version.translated_summary.clone();
                document.chunks = chunks_by_version
                    .get(&version.id)
                    .map(|chunks| {
                        chunks
                            .iter()
                            .map(|chunk| DocumentChunkResponse {
                                chunk_id: chunk.id,
                                chunk_index: chunk.chunk_index,
                                text: chunk.chunk_text.clone(),
                                record_hash: version.source_record_hash.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                document.requested_locale = Some(locale.to_string());
                document.content_locale = Some(version.target_locale.clone());
                document.translation_status = Some(TranslationStatus::Succeeded);
                document.is_fallback = false;
                continue;
            }

            let status = statuses_by_document.get(document_id);
            document.requested_locale = Some(locale.to_string());
            document.content_locale = status
                .and_then(|value| value.source_locale.clone())
                .or_else(|| Some("original".to_string()));
            document.translation_status = Some(match status.map(|value| value.status.as_str()) {
                Some("queued") => TranslationStatus::Queued,
                Some("running") => TranslationStatus::Running,
                Some("failed") => TranslationStatus::Failed,
                Some("skipped") => TranslationStatus::Skipped,
                Some("quota_exceeded") => TranslationStatus::QuotaExceeded,
                Some("succeeded") => TranslationStatus::Succeeded,
                _ => TranslationStatus::Unavailable,
            });
            document.is_fallback = true;
        }

        Ok(result)
    }

    pub async fn document_chunk_ids(&self, document_id: i64) -> Result<Vec<Uuid>> {
        self.list_chunk_ids_for_document(document_id).await
    }

    pub async fn delete_document_by_id(&self, document_id: i64) -> Result<()> {
        super::entity::documents::Entity::delete_by_id(document_id)
            .exec(&self.orm)
            .await?;
        Ok(())
    }
    pub async fn upsert_document(&self, payload: &ChunkPayload) -> Result<UpsertedDocument> {
        let mut tx = self.pool.begin().await?;
        let (document_id, changed) = upsert_document_row_in(&mut tx, payload, None).await?;

        tx.commit().await?;
        let mut metadata_keys = Vec::new();
        let mut metadata_values = Vec::new();
        for definition in self
            .list_metadata_indexes(payload.group_id, &payload.source_key)
            .await?
            .into_iter()
            .filter(|definition| definition.status == "ready")
        {
            metadata_keys.push((definition.index_id, document_id));
            let values = crate::services::document_store::metadata::extract_values(
                &definition,
                &payload.metadata_json,
            )?;
            metadata_values.extend(crate::db::metadata_indexes::metadata_value_rows(
                definition.index_id,
                document_id,
                &values,
            ));
        }
        self.replace_metadata_values_bulk(&metadata_keys, &metadata_values)
            .await?;
        Ok(UpsertedDocument {
            document_id,
            changed,
        })
    }

    /// The document, its version and its metadata values, on a borrowed
    /// connection.
    ///
    /// A task-owned write runs the three statements on the connection of its
    /// guarded transaction, so the document, the version that describes it and
    /// the metadata values that index it commit together.
    ///
    /// A reserved id that names a tombstoned document is skipped: the document
    /// is deleted, so a new document with that id would be invisible to every
    /// search that still excludes the tombstone. The skip is bounded, and a
    /// run of tombstoned candidates is refused instead of consuming the
    /// sequence without an end.
    pub async fn reserve_document_id_on(&self, connection: &mut sqlx::PgConnection) -> Result<i64> {
        for _ in 0..RESERVED_DOCUMENT_ID_TOMBSTONE_SKIP_LIMIT {
            let reserved = sqlx::query_file_scalar!("src/sql/db/documents/reserve_document_id.sql")
                .fetch_optional(&mut *connection)
                .await?;
            if let Some(id) = reserved {
                return Ok(id);
            }
        }
        anyhow::bail!(
            "the document id sequence produced {RESERVED_DOCUMENT_ID_TOMBSTONE_SKIP_LIMIT} \
             tombstoned ids in a row; the reservation is refused"
        )
    }

    pub async fn upsert_document_in(
        &self,
        connection: &mut sqlx::PgConnection,
        payload: &ChunkPayload,
    ) -> Result<UpsertedDocument> {
        self.upsert_document_with_id_in(connection, payload, None)
            .await
    }

    pub async fn upsert_document_with_reserved_id_in(
        &self,
        connection: &mut sqlx::PgConnection,
        payload: &ChunkPayload,
        document_id: i64,
    ) -> Result<UpsertedDocument> {
        self.upsert_document_with_id_in(connection, payload, Some(document_id))
            .await
    }

    async fn upsert_document_with_id_in(
        &self,
        connection: &mut sqlx::PgConnection,
        payload: &ChunkPayload,
        document_id: Option<i64>,
    ) -> Result<UpsertedDocument> {
        let (document_id, changed) =
            upsert_document_row_in(connection, payload, document_id).await?;
        let definitions = crate::db::metadata_indexes::list_metadata_indexes_in(
            connection,
            payload.group_id,
            &payload.source_key,
        )
        .await?;
        let mut metadata_keys = Vec::new();
        let mut metadata_values = Vec::new();
        for definition in definitions
            .into_iter()
            .filter(|definition| definition.status == "ready")
        {
            metadata_keys.push((definition.index_id, document_id));
            let values = crate::services::document_store::metadata::extract_values(
                &definition,
                &payload.metadata_json,
            )?;
            metadata_values.extend(crate::db::metadata_indexes::metadata_value_rows(
                definition.index_id,
                document_id,
                &values,
            ));
        }
        crate::db::metadata_indexes::replace_metadata_values_in_connection(
            connection,
            &metadata_keys,
            &metadata_values,
        )
        .await?;
        Ok(UpsertedDocument {
            document_id,
            changed,
        })
    }

    pub async fn update_library_document_business_fields(
        &self,
        document_id: i64,
        payload: &ChunkPayload,
    ) -> Result<()> {
        let definitions = self
            .list_metadata_indexes(payload.group_id, &payload.source_key)
            .await?
            .into_iter()
            .filter(|definition| definition.status == "ready")
            .map(|definition| {
                let values = crate::services::document_store::metadata::extract_values(
                    &definition,
                    &payload.metadata_json,
                )?;
                Ok((definition.index_id, values))
            })
            .collect::<Result<Vec<_>>>()?;

        let mut tx = self.pool.begin().await?;
        let Some(expected) = self
            .lock_replacement_documents_on(&mut tx, &[document_id])
            .await?
            .into_iter()
            .next()
            .map(|row| row.record_hash)
        else {
            return Ok(());
        };
        super::document_versions::ensure_library_version_in_transaction(
            &mut tx,
            document_id,
            payload,
        )
        .await?;
        if !apply_business_fields_on(&mut tx, document_id, payload, &expected).await? {
            anyhow::bail!(
                "document {document_id} changed while its business fields were published"
            );
        }
        let metadata_keys = definitions
            .iter()
            .map(|(index_id, _)| (*index_id, document_id))
            .collect::<Vec<_>>();
        let metadata_values = definitions
            .into_iter()
            .flat_map(|(index_id, values)| {
                crate::db::metadata_indexes::metadata_value_rows(index_id, document_id, &values)
            })
            .collect::<Vec<_>>();
        crate::db::metadata_indexes::replace_metadata_values_in_transaction(
            &mut tx,
            &metadata_keys,
            &metadata_values,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The business-fields publish of one document on a caller transaction.
    ///
    /// `body_text` is the canonical body the caller hashed; the version
    /// snapshot stores that exact text. `expected_record_hash` is enforced by
    /// the UPDATE predicate, so a baseline that moved refuses with `false` and
    /// the caller rolls its whole file/document transaction back.
    pub async fn update_library_document_business_fields_in(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
        payload: &ChunkPayload,
        expected_record_hash: &str,
        body_text: &str,
    ) -> Result<bool> {
        super::document_versions::ensure_library_version_in_connection(
            connection,
            document_id,
            payload,
            body_text,
        )
        .await?;
        if !apply_business_fields_on(connection, document_id, payload, expected_record_hash).await?
        {
            return Ok(false);
        }
        let definitions = crate::db::metadata_indexes::list_metadata_indexes_in(
            connection,
            payload.group_id,
            &payload.source_key,
        )
        .await?;
        let mut metadata_keys = Vec::new();
        let mut metadata_values = Vec::new();
        for definition in definitions
            .into_iter()
            .filter(|definition| definition.status == "ready")
        {
            metadata_keys.push((definition.index_id, document_id));
            let values = crate::services::document_store::metadata::extract_values(
                &definition,
                &payload.metadata_json,
            )?;
            metadata_values.extend(crate::db::metadata_indexes::metadata_value_rows(
                definition.index_id,
                document_id,
                &values,
            ));
        }
        crate::db::metadata_indexes::replace_metadata_values_in_connection(
            connection,
            &metadata_keys,
            &metadata_values,
        )
        .await?;
        Ok(true)
    }

    pub async fn replace_document_chunks(
        &self,
        document_id: i64,
        record_hash: &str,
        chunks: &[DocumentChunk],
    ) -> Result<Vec<Uuid>> {
        let mut tx = self.pool.begin().await?;
        // A rebuild replaces the live chunk set of every document, so a sync
        // write must not add chunks to the collection being re-embedded.
        self.require_rebuild_idle_in(&mut tx).await?;
        sqlx::query_file!(
            "src/sql/db/documents/delete_document_chunks.sql",
            document_id
        )
        .execute(&mut *tx)
        .await?;

        insert_document_chunks_in_transaction(&mut tx, document_id, record_hash, chunks).await?;

        tx.commit().await?;
        Ok(chunks.iter().map(|chunk| chunk.id).collect())
    }

    pub async fn delete_document_chunks(&self, document_id: i64) -> Result<()> {
        use super::entity::document_chunks::{Column, Entity};
        Entity::delete_many()
            .filter(Column::DocumentId.eq(document_id))
            .exec(&self.orm)
            .await?;
        Ok(())
    }

    pub async fn insert_document_chunks(
        &self,
        document_id: i64,
        record_hash: &str,
        chunks: &[DocumentChunk],
    ) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        insert_document_chunks_in_transaction(&mut tx, document_id, record_hash, chunks).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The chunks of one document, inserted on a borrowed connection.
    ///
    /// A task-owned write inserts the chunks on the connection of its guarded
    /// transaction, so the chunks and the grant that authorized them commit
    /// together.
    pub async fn insert_document_chunks_in(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
        record_hash: &str,
        chunks: &[DocumentChunk],
    ) -> Result<()> {
        insert_document_chunks_on(connection, document_id, record_hash, chunks).await
    }

    /// Verify that every expected chunk id of one batch is present with the
    /// exact document, index, record hash and text of the batch.
    ///
    /// A primary-key conflict is only idempotent when the stored row is the
    /// same chunk. A row with the same id but a different index, hash or text
    /// would mix two segmentations, so the batch refuses instead of advancing
    /// its checkpoint.
    pub async fn verify_document_chunk_batch_in(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
        chunks: &[DocumentChunk],
    ) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let chunk_ids = chunks.iter().map(|chunk| chunk.id).collect::<Vec<_>>();
        let chunk_indexes = chunks
            .iter()
            .map(|chunk| chunk.chunk_index)
            .collect::<Vec<_>>();
        let chunk_texts = chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let record_hashes = chunks
            .iter()
            .map(|chunk| chunk.record_hash.clone())
            .collect::<Vec<_>>();
        let mismatches: i64 = sqlx::query_file_scalar!(
            "src/sql/db/documents/verify_document_chunk_batch.sql",
            document_id,
            &chunk_ids,
            &chunk_indexes,
            &chunk_texts,
            &record_hashes
        )
        .fetch_one(connection)
        .await
        .context("verify document chunk batch")?;
        anyhow::ensure!(
            mismatches == 0,
            "the persisted chunk batch does not match the expected rows; the checkpoint cannot advance"
        );
        Ok(())
    }

    /// The chunks of one document, removed on a borrowed connection.
    pub async fn delete_document_chunks_in(
        &self,
        connection: &mut sqlx::PgConnection,
        document_id: i64,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/documents/delete_document_chunks.sql",
            document_id
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    pub async fn list_chunk_ids_for_document(&self, document_id: i64) -> Result<Vec<Uuid>> {
        use sea_orm::{
            ConnectionTrait,
            sea_query::{Alias, Expr, ExprTrait, Query, UnionType},
        };
        let translation = Query::select()
            .column(Alias::new("id"))
            .from((
                Alias::new("context69"),
                Alias::new("document_translation_chunks"),
            ))
            .and_where(Expr::col(Alias::new("document_id")).eq(document_id))
            .to_owned();
        let query = Query::select()
            .column(Alias::new("id"))
            .from((Alias::new("context69"), Alias::new("document_chunks")))
            .and_where(Expr::col(Alias::new("document_id")).eq(document_id))
            .union(UnionType::All, translation)
            .to_owned();
        self.orm
            .query_all(&query)
            .await?
            .into_iter()
            .map(|row| row.try_get("", "id").map_err(Into::into))
            .collect()
    }

    pub async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        requested_locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>> {
        let rows = sqlx::query_file_as!(
            SearchHitRow,
            "src/sql/db/documents/fetch_search_hits_by_chunk_ids.sql",
            chunk_ids,
            &scope.private_group_ids,
            scope.group_path.as_deref(),
            requested_locale
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.chunk_id,
                    SearchHit {
                        chunk_id: row.chunk_id,
                        document_id: row.document_id,
                        group_key: row.group_key,
                        group_path: row.group_path,
                        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
                        source_key: row.source_key,
                        external_id: row.external_id,
                        title: row.title,
                        summary: row.summary.filter(|value| is_meaningful_text(value)),
                        source_uri: row.source_uri,
                        published_at: row.published_at,
                        chunk_index: row.chunk_index,
                        chunk_text: row.chunk_text,
                        score: 0.0,
                        vector_score: None,
                        keyword_score: None,
                        rerank_score: None,
                        match_reason: None,
                        library_file_id: library_file_id(&row.metadata_json),
                        library_section_label: library_section_label(&row.metadata_json),
                        library_path: library_path(&row.metadata_json),
                        is_library_file: is_library_file(&row.metadata_json),
                        metadata_json: row.metadata_json,
                        requested_locale: requested_locale.map(ToOwned::to_owned),
                        content_locale: Some(row.content_locale.clone()),
                        translation_status: translation_status(row.translation_status.as_deref()),
                        is_fallback: requested_locale.is_some() && row.content_locale == "original",
                    },
                )
            })
            .collect())
    }

    pub async fn keyword_search(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let normalized_query = request.query.trim().to_lowercase();
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }
        let phrase_pattern = format!("%{normalized_query}%");
        let terms = keyword_terms(&normalized_query);
        let metadata_filters = serde_json::to_value(&request.metadata_filters)?;
        let keyword_limit = i64::try_from(limit).context("keyword search limit is too large")?;

        let rows = sqlx::query_file_as!(
            KeywordSearchHitRow,
            "src/sql/db/documents/keyword_search.sql",
            normalized_query,
            phrase_pattern,
            &terms,
            request.source_key,
            request.group_path.as_deref(),
            request.published_after,
            request.published_before,
            &scope.private_group_ids,
            keyword_limit,
            request.locale.as_deref(),
            metadata_filters
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| search_hit_from_keyword_row(row, request.locale.as_deref()))
            .collect())
    }

    pub async fn count_chunk_payloads_for_reindex(&self) -> Result<usize> {
        let count =
            sqlx::query_file_scalar!("src/sql/db/documents/count_chunk_payloads_for_reindex.sql")
                .fetch_one(&self.pool)
                .await?;
        usize::try_from(count).context("reindex chunk count is out of range")
    }

    pub async fn list_chunk_payloads_for_reindex_page(
        &self,
        last_document_id: Option<i64>,
        last_chunk_index: Option<i32>,
        limit: i64,
    ) -> Result<Vec<ChunkPayload>> {
        let rows = sqlx::query_file_as!(
            ReindexChunkRow,
            "src/sql/db/documents/list_chunk_payloads_for_reindex_page.sql",
            last_document_id,
            last_chunk_index,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(reindex_payload_from_row).collect())
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<Option<DocumentResponse>> {
        let document = sqlx::query_file_as!(
            DocumentRow,
            "src/sql/db/documents/get_document.sql",
            document_id,
            &scope.private_group_ids,
            scope.group_path.as_deref()
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(document) = document else {
            return Ok(None);
        };

        let chunks = sqlx::query_file_as!(
            ReadDocumentChunkRow,
            "src/sql/db/documents/get_read_document_chunks.sql",
            document_id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(document_response_from_parts(
            document,
            chunks
                .into_iter()
                .filter(|chunk| is_meaningful_text(&chunk.chunk_text))
                .map(|chunk| DocumentChunkResponse {
                    chunk_id: chunk.id,
                    chunk_index: chunk.chunk_index,
                    text: chunk.chunk_text,
                    record_hash: chunk.record_hash,
                })
                .collect(),
            None,
            None,
            false,
        )))
    }
}

fn document_response_from_parts(
    document: DocumentRow,
    chunks: Vec<DocumentChunkResponse>,
    requested_locale: Option<String>,
    content_locale: Option<String>,
    is_fallback: bool,
) -> DocumentResponse {
    DocumentResponse {
        document_id: document.id,
        group_key: document.group_key,
        group_path: document.group_path,
        visibility: document.visibility.parse().unwrap_or(Visibility::Private),
        source_key: document.source_key,
        external_id: document.external_id,
        title: document.title,
        summary: document.summary.filter(|value| is_meaningful_text(value)),
        source_uri: document.source_uri,
        published_at: document.published_at,
        updated_at: document.updated_at_source,
        record_hash: document.record_hash,
        library_file_id: library_file_id(&document.metadata_json),
        library_section_label: library_section_label(&document.metadata_json),
        library_path: library_path(&document.metadata_json),
        is_library_file: is_library_file(&document.metadata_json),
        metadata_json: document.metadata_json,
        chunks,
        requested_locale,
        content_locale,
        translation_status: None,
        is_fallback,
    }
}

fn reindex_payload_from_row(row: ReindexChunkRow) -> ChunkPayload {
    from_row(RowPayload {
        chunk_id: row.chunk_id,
        document_id: row.document_id,
        group_id: row.group_id,
        group_key: row.group_key,
        group_path: row.group_path,
        visibility: row.visibility,
        source_key: row.source_key,
        external_id: row.external_id,
        title: row.title,
        summary: row.summary,
        source_uri: row.source_uri,
        published_at: row.published_at,
        updated_at_source: row.updated_at_source,
        record_hash: row.record_hash,
        chunk_index: row.chunk_index,
        chunk_text: row.chunk_text,
        metadata_json: row.metadata_json,
    })
}

/// The document row and its version, on a borrowed connection.
///
/// The version is written only when the record changed, exactly as the pooled
/// write does, so an unchanged document keeps its version history.
async fn upsert_document_row_in(
    connection: &mut sqlx::PgConnection,
    payload: &ChunkPayload,
    reserved_id: Option<i64>,
) -> Result<(i64, bool)> {
    let mut existing = sqlx::query_file_as!(
        ExistingDocumentRow,
        "src/sql/db/documents/get_existing_document.sql",
        payload.group_id,
        payload.source_key,
        payload.external_id
    )
    .fetch_optional(&mut *connection)
    .await?;

    if let Some(reserved_id) = reserved_id {
        if let Some(found) = &existing {
            anyhow::ensure!(
                found.id == reserved_id,
                "document identity does not match reserved id"
            );
        }
        let locked = sqlx::query_file_as!(
            ReplacementDocumentSnapshot,
            "src/sql/db/documents/lock_replacement_documents.sql",
            &[reserved_id]
        )
        .fetch_optional(&mut *connection)
        .await?;
        if let Some(locked) = locked {
            anyhow::ensure!(
                locked.group_id == payload.group_id && locked.source_key == payload.source_key,
                "reserved document belongs to another group or source"
            );
            if locked.external_id != payload.external_id {
                anyhow::ensure!(
                    existing.is_none(),
                    "replacement document key already exists"
                );
                anyhow::ensure!(
                    apply_business_fields_on(
                        &mut *connection,
                        reserved_id,
                        payload,
                        &locked.record_hash,
                    )
                    .await?,
                    "reserved document changed while its business fields were updated"
                );
                existing = Some(ExistingDocumentRow {
                    id: locked.id,
                    record_hash: locked.record_hash,
                });
            }
        }
    }

    let (document_id, changed) = match existing {
        Some(existing) if existing.record_hash == payload.record_hash => {
            sqlx::query_file!(
                "src/sql/db/documents/update_document_unchanged.sql",
                payload.group_id,
                payload.source_key,
                payload.title,
                payload.summary,
                payload.source_uri,
                payload.published_at,
                payload.updated_at_source,
                payload.metadata_json,
                payload.external_id
            )
            .execute(&mut *connection)
            .await?;
            (existing.id, false)
        }
        Some(existing) => {
            sqlx::query_file!(
                "src/sql/db/documents/update_document_changed.sql",
                payload.group_id,
                payload.source_key,
                payload.title,
                payload.summary,
                payload.source_uri,
                payload.published_at,
                payload.updated_at_source,
                payload.metadata_json,
                payload.record_hash,
                payload.external_id
            )
            .execute(&mut *connection)
            .await?;
            (existing.id, true)
        }
        None if reserved_id.is_some() => {
            let id = sqlx::query_file_scalar!(
                "src/sql/db/documents/insert_document_reserved.sql",
                reserved_id,
                payload.group_id,
                payload.visibility.as_str(),
                payload.source_key,
                payload.external_id,
                payload.title,
                payload.summary,
                payload.source_uri,
                payload.published_at,
                payload.updated_at_source,
                payload.metadata_json,
                payload.record_hash
            )
            .fetch_one(&mut *connection)
            .await?;
            (id, true)
        }
        None => {
            let id = sqlx::query_file_scalar!(
                "src/sql/db/documents/insert_document.sql",
                payload.group_id,
                payload.visibility.as_str(),
                payload.source_key,
                payload.external_id,
                payload.title,
                payload.summary,
                payload.source_uri,
                payload.published_at,
                payload.updated_at_source,
                payload.metadata_json,
                payload.record_hash
            )
            .fetch_one(&mut *connection)
            .await?;
            (id, true)
        }
    };

    if changed {
        sqlx::query_file!(
            "src/sql/db/documents/insert_document_version.sql",
            document_id,
            payload.record_hash,
            payload.title,
            payload.summary,
            payload.chunk_text,
            payload.source_uri,
            payload.published_at,
            payload.metadata_json
        )
        .execute(&mut *connection)
        .await?;
    }

    Ok((document_id, changed))
}

/// Rewrite the business fields and the chunk hashes of one document.
///
/// The document UPDATE is a compare-and-set on the expected current hash and
/// returns the id only when it matched. The chunk rewrite targets only that
/// document, so a CAS miss touches nothing.
async fn apply_business_fields_on(
    connection: &mut sqlx::PgConnection,
    document_id: i64,
    payload: &ChunkPayload,
    expected_record_hash: &str,
) -> Result<bool> {
    let updated = sqlx::query_file_scalar!(
        "src/sql/db/documents/update_library_business_fields.sql",
        document_id,
        payload.external_id,
        payload.source_uri,
        payload.published_at,
        payload.updated_at_source,
        payload.metadata_json,
        payload.record_hash,
        expected_record_hash
    )
    .fetch_optional(&mut *connection)
    .await?;
    if updated.is_none() {
        return Ok(false);
    }
    sqlx::query_file!(
        "src/sql/db/documents/update_library_chunk_record_hash.sql",
        document_id,
        payload.record_hash
    )
    .execute(&mut *connection)
    .await?;
    Ok(true)
}

async fn insert_document_chunks_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: i64,
    record_hash: &str,
    chunks: &[DocumentChunk],
) -> Result<()> {
    insert_document_chunks_on(tx, document_id, record_hash, chunks).await
}

async fn insert_document_chunks_on(
    connection: &mut sqlx::PgConnection,
    document_id: i64,
    record_hash: &str,
    chunks: &[DocumentChunk],
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    let chunk_ids = chunks.iter().map(|chunk| chunk.id).collect::<Vec<_>>();
    let chunk_indexes = chunks
        .iter()
        .map(|chunk| chunk.chunk_index)
        .collect::<Vec<_>>();
    let chunk_texts = chunks
        .iter()
        .map(|chunk| chunk.text.clone())
        .collect::<Vec<_>>();
    sqlx::query_file!(
        "src/sql/db/documents/insert_document_chunks_bulk.sql",
        document_id,
        record_hash,
        &chunk_ids,
        &chunk_indexes,
        &chunk_texts
    )
    .execute(connection)
    .await?;
    Ok(())
}
