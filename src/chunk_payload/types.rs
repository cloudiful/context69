//! Named-field inputs and support types for the payload constructors.
//!
//! Grouping the varying payload fields into dedicated structs keeps every
//! constructor call site readable and free of positional argument lists. The
//! persistence mapper's owned [`RowPayload`] stays domain-only, so the shared
//! mapping never couples to database row types.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::contracts::Visibility;

/// Group/namespace ownership shared by every payload of a source.
pub(crate) struct PayloadGroup<'a> {
    pub group_id: i64,
    pub group_key: &'a str,
    pub group_path: &'a str,
    pub visibility: Visibility,
}

/// Source-document fields shared by every payload of a document.
pub(crate) struct PayloadDocument<'a> {
    pub source_key: &'a str,
    pub external_id: &'a str,
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub source_uri: &'a str,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub record_hash: &'a str,
    pub metadata_json: &'a Value,
}

/// Position and text of one chunk within a document.
pub(crate) struct ChunkRef<'a> {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub chunk_index: i32,
    pub chunk_text: &'a str,
}

impl<'a> ChunkRef<'a> {
    /// Placeholder used before `upsert_document` assigns a document id.
    pub(super) fn seed(body_text: &'a str) -> Self {
        Self {
            chunk_id: Uuid::nil(),
            document_id: 0,
            chunk_index: 0,
            chunk_text: body_text,
        }
    }

    /// Document-level position used by business-field updates.
    pub(super) fn document_level(document_id: i64, body_text: &'a str) -> Self {
        Self {
            chunk_id: Uuid::nil(),
            document_id,
            chunk_index: 0,
            chunk_text: body_text,
        }
    }
}

/// Locale metadata carried by a payload; `None` means original content.
#[derive(Clone, Copy)]
pub(crate) struct PayloadLocale<'a> {
    pub content_locale: &'a str,
    pub source_locale: Option<&'a str>,
    pub translation_provider: Option<&'a str>,
}

/// Owned payload fields read from persistence, before locale defaults are
/// applied. Kept free of database row types so the shared mapping stays
/// domain-only while still moving values out of a row.
pub(crate) struct RowPayload {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: String,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub record_hash: String,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub metadata_json: Value,
}
