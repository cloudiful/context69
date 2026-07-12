use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::contracts::{LibraryIngestStatus, Visibility};

pub use context69_namespace::{
    AccessScope, GroupRecord, NamespaceMemberRecord, PersonalGroupRecord,
};

#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub external_id: String,
    pub title: String,
    pub body_text: String,
    pub source_uri: String,
    pub summary: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub metadata_json: Value,
}

#[derive(Debug, Clone)]
pub struct NormalizedDocument {
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub body_text: String,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub metadata_json: Value,
    pub record_hash: String,
}

#[derive(Debug, Clone)]
pub struct DocumentChunk {
    pub id: Uuid,
    pub document_id: i64,
    pub chunk_index: i32,
    pub text: String,
    pub record_hash: String,
}

#[derive(Debug, Clone)]
pub struct SyncCheckpoint {
    pub updated_at: Option<DateTime<Utc>>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChunkPayload {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
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
    pub content_locale: String,
    pub source_locale: Option<String>,
    pub translation_provider: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceMetadata {
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct LibraryFolderRecord {
    pub id: Uuid,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct LibraryFileRecord {
    pub id: Uuid,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub source_uri: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub metadata_json: Value,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub ingest_status: LibraryIngestStatus,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub ingested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct LibraryIngestJobRecord {
    pub id: Uuid,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub file_id: Uuid,
    pub status: LibraryIngestStatus,
    pub docling_task_id: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct LibraryFileDocumentRecord {
    pub file_id: Uuid,
    pub document_id: i64,
    pub group_id: i64,
    pub visibility: Visibility,
    pub section_key: String,
    pub section_label: String,
    pub section_external_id: Option<String>,
    pub section_source_uri: Option<String>,
    pub section_published_at: Option<DateTime<Utc>>,
    pub section_metadata_json: Value,
    pub sort_order: i32,
}

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: i64,
    pub login_name: String,
    pub display_name: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
