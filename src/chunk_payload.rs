//! Crate-private constructors for [`ChunkPayload`].
//!
//! Every ingest, sync, extraction, translation and reindex path repeats the
//! same group/scope and source-document fields; only the chunk position and
//! locale metadata vary. Centralising them here keeps the field mapping in one
//! audited place while preserving the public [`ChunkPayload`] shape.
//!
//! The named-field inputs live in [`types`]; this module owns the semantic
//! constructors, the shared field literal and the persistence-row mapper.

mod types;

#[cfg(test)]
mod tests;

pub(crate) use types::{ChunkRef, PayloadDocument, PayloadGroup, PayloadLocale, RowPayload};

use crate::contracts::Visibility;
use crate::domain::ChunkPayload;

/// `content_locale` of a payload that was not produced by translation.
const ORIGINAL_LOCALE: &str = "original";

/// Seed payload for `upsert_document`: neither the document id nor the chunk
/// id exists yet, so both carry the placeholder values the upsert expects.
pub(crate) fn seed(
    group: &PayloadGroup<'_>,
    document: &PayloadDocument<'_>,
    body_text: &str,
) -> ChunkPayload {
    original_chunk(group, document, ChunkRef::seed(body_text))
}

/// Document-level payload for updating the business fields of a persisted
/// document, using its stored body text as the single chunk.
pub(crate) fn business_update(
    group: &PayloadGroup<'_>,
    document: &PayloadDocument<'_>,
    document_id: i64,
    body_text: &str,
) -> ChunkPayload {
    original_chunk(
        group,
        document,
        ChunkRef::document_level(document_id, body_text),
    )
}

/// Original-locale payload for a concrete persisted chunk.
pub(crate) fn original_chunk(
    group: &PayloadGroup<'_>,
    document: &PayloadDocument<'_>,
    chunk: ChunkRef<'_>,
) -> ChunkPayload {
    with_locale(
        group,
        document,
        chunk,
        PayloadLocale {
            content_locale: ORIGINAL_LOCALE,
            source_locale: None,
            translation_provider: None,
        },
    )
}

/// Translated payload carrying the target locale and provider.
pub(crate) fn translated_chunk(
    group: &PayloadGroup<'_>,
    document: &PayloadDocument<'_>,
    chunk: ChunkRef<'_>,
    locale: PayloadLocale<'_>,
) -> ChunkPayload {
    with_locale(group, document, chunk, locale)
}

fn with_locale(
    group: &PayloadGroup<'_>,
    document: &PayloadDocument<'_>,
    chunk: ChunkRef<'_>,
    locale: PayloadLocale<'_>,
) -> ChunkPayload {
    ChunkPayload {
        chunk_id: chunk.chunk_id,
        document_id: chunk.document_id,
        group_id: group.group_id,
        group_key: group.group_key.to_string(),
        group_path: group.group_path.to_string(),
        visibility: group.visibility,
        source_key: document.source_key.to_string(),
        external_id: document.external_id.to_string(),
        title: document.title.to_string(),
        summary: document.summary.map(str::to_string),
        source_uri: document.source_uri.to_string(),
        published_at: document.published_at,
        updated_at_source: document.updated_at_source,
        record_hash: document.record_hash.to_string(),
        chunk_index: chunk.chunk_index,
        chunk_text: chunk.chunk_text.to_string(),
        metadata_json: document.metadata_json.clone(),
        content_locale: locale.content_locale.to_string(),
        source_locale: locale.source_locale.map(str::to_string),
        translation_provider: locale.translation_provider.map(str::to_string),
    }
}

/// Map a persisted chunk row into an original-locale payload.
pub(crate) fn from_row(row: RowPayload) -> ChunkPayload {
    ChunkPayload {
        chunk_id: row.chunk_id,
        document_id: row.document_id,
        group_id: row.group_id,
        group_key: row.group_key,
        group_path: row.group_path,
        visibility: visibility_or_private(&row.visibility),
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
        content_locale: ORIGINAL_LOCALE.to_string(),
        source_locale: None,
        translation_provider: None,
    }
}

/// Parse a persisted visibility, defaulting unknown values to `Private`.
pub(crate) fn visibility_or_private(raw: &str) -> Visibility {
    raw.parse().unwrap_or(Visibility::Private)
}
