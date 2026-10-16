use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;

use super::Database;
use crate::contracts::{
    CreateDocumentResultRequest, DocumentResultProvenance, DocumentResultResponse,
    ListDocumentResultsQuery, ListDocumentResultsResponse,
};
use crate::domain::AccessScope;
use crate::domain_errors::DomainError;

#[derive(Debug, Clone, FromRow)]
struct DocumentResultRow {
    id: i64,
    document_id: i64,
    author_user_id: i64,
    producer: String,
    result_kind: String,
    schema_ref: String,
    source_record_hash: String,
    idempotency_key: String,
    result: Value,
    provenance: Option<Value>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct DocumentTargetRow {
    record_hash: String,
}

impl DocumentResultRow {
    fn into_response(self) -> Result<DocumentResultResponse> {
        Ok(DocumentResultResponse {
            id: self.id,
            document_id: self.document_id,
            author_user_id: self.author_user_id,
            producer: self.producer,
            result_kind: self.result_kind,
            schema_ref: self.schema_ref,
            source_record_hash: self.source_record_hash,
            idempotency_key: self.idempotency_key,
            result: self.result,
            provenance: decode_provenance(self.provenance)?,
            created_at: self.created_at,
        })
    }
}

fn decode_provenance(value: Option<Value>) -> Result<Option<DocumentResultProvenance>> {
    match value {
        Some(value) => {
            let provenance = serde_json::from_value(value)
                .context("stored document result provenance is not a valid object")?;
            Ok(Some(provenance))
        }
        None => Ok(None),
    }
}

fn encode_provenance(provenance: Option<&DocumentResultProvenance>) -> Result<Option<Value>> {
    match provenance {
        Some(provenance) => Ok(Some(serde_json::to_value(provenance)?)),
        None => Ok(None),
    }
}

impl Database {
    /// Append one caller-produced result, or replay the immutable row already
    /// stored under the same `(document, author, producer, idempotency_key)`.
    ///
    /// The document row is locked for the whole transaction, which serializes
    /// against a concurrent document update carrying a new `record_hash`. The
    /// replay check runs before the source-hash check, so an accepted delivery
    /// stays safely retryable after a later document update; a new key against
    /// a moved-on document is a conflict.
    pub async fn append_document_result(
        &self,
        group_id: i64,
        document_id: i64,
        group_path: &str,
        author_user_id: i64,
        scope: &AccessScope,
        request: &CreateDocumentResultRequest,
    ) -> Result<DocumentResultResponse> {
        let mut tx = self.pool.begin().await?;

        let target = sqlx::query_file_as!(
            DocumentTargetRow,
            "src/sql/db/document_results/lock_document_for_result.sql",
            document_id,
            group_id,
            &scope.private_group_ids,
            group_path
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DomainError::not_found("document not found"))?;

        let replay = sqlx::query_file_as!(
            DocumentResultRow,
            "src/sql/db/document_results/get_replay.sql",
            document_id,
            author_user_id,
            request.producer,
            request.idempotency_key
        )
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(existing) = replay {
            if !replay_matches(&existing, request)? {
                return Err(DomainError::conflict(
                    "idempotency key was already used with a different result envelope",
                )
                .into());
            }
            let response = existing.into_response()?;
            tx.commit().await?;
            return Ok(response);
        }

        if target.record_hash != request.source_record_hash {
            return Err(DomainError::conflict(
                "source_record_hash does not match the current document version",
            )
            .into());
        }

        let provenance = encode_provenance(request.provenance.as_ref())?;
        let inserted = sqlx::query_file_as!(
            DocumentResultRow,
            "src/sql/db/document_results/insert_result.sql",
            document_id,
            author_user_id,
            request.producer,
            request.result_kind,
            request.schema_ref,
            request.source_record_hash,
            request.idempotency_key,
            request.result,
            provenance
        )
        .fetch_one(&mut *tx)
        .await?;

        let response = inserted.into_response()?;
        tx.commit().await?;
        Ok(response)
    }

    /// List one document's appended results newest-first, scoped to the caller.
    ///
    /// The read reports the document's current source hash so a caller can bind
    /// a subsequent append, and it never mutates document metadata.
    pub async fn list_document_results(
        &self,
        group_id: i64,
        document_id: i64,
        group_path: &str,
        scope: &AccessScope,
        query: &ListDocumentResultsQuery,
    ) -> Result<ListDocumentResultsResponse> {
        query
            .validate()
            .map_err(|error| DomainError::invalid_argument(error.to_string()))?;

        let target = sqlx::query_file_as!(
            DocumentTargetRow,
            "src/sql/db/document_results/get_document_result_target.sql",
            document_id,
            group_id,
            &scope.private_group_ids,
            group_path
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DomainError::not_found("document not found"))?;

        let limit = i64::from(query.limit);
        let fetch_limit = limit + 1;
        let rows = sqlx::query_file_as!(
            DocumentResultRow,
            "src/sql/db/document_results/list_results.sql",
            document_id,
            query.producer,
            query.result_kind,
            query.after_id,
            fetch_limit
        )
        .fetch_all(&self.pool)
        .await?;

        let mut items = rows
            .into_iter()
            .map(DocumentResultRow::into_response)
            .collect::<Result<Vec<_>>>()?;
        let next_after_id = if items.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
            items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            items.last().map(|item| item.id)
        } else {
            None
        };

        Ok(ListDocumentResultsResponse {
            source_record_hash: target.record_hash,
            items,
            next_after_id,
        })
    }
}

/// Semantic envelope equality for a replay: the immutable identity and the
/// opaque payload and provenance must all agree. `serde_json::Value` equality is
/// key-order independent, and provenance is compared as its stored JSON.
fn replay_matches(row: &DocumentResultRow, request: &CreateDocumentResultRequest) -> Result<bool> {
    if row.producer != request.producer
        || row.result_kind != request.result_kind
        || row.schema_ref != request.schema_ref
        || row.source_record_hash != request.source_record_hash
        || row.result != request.result
    {
        return Ok(false);
    }
    let requested_provenance = encode_provenance(request.provenance.as_ref())?;
    Ok(row.provenance == requested_provenance)
}
