use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::contracts::{GroupKind, LibraryIngestStatus, MembershipRole, Visibility};

#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub external_id: String,
    pub title: String,
    pub body_text: String,
    pub source_uri: String,
    pub summary: Option<String>,
    pub published_at: Option<NaiveDate>,
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
    pub published_at: Option<NaiveDate>,
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
    pub project_id: i64,
    pub project_key: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<NaiveDate>,
    pub updated_at_source: DateTime<Utc>,
    pub record_hash: String,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub metadata_json: Value,
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
    pub project_id: i64,
    pub project_key: String,
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
    pub project_id: i64,
    pub project_key: String,
    pub visibility: Visibility,
    pub folder_id: Option<Uuid>,
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
    pub project_id: i64,
    pub project_key: String,
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
    pub project_id: i64,
    pub visibility: Visibility,
    pub section_key: String,
    pub section_label: String,
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

#[derive(Debug, Clone)]
pub struct GroupRecord {
    pub id: i64,
    pub parent_group_id: Option<i64>,
    pub parent_group_key: Option<String>,
    pub group_key: String,
    pub name: String,
    pub visibility: Visibility,
    pub kind: GroupKind,
    pub owner_user_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_role: Option<MembershipRole>,
}

#[derive(Debug, Clone)]
pub struct ProjectRecord {
    pub id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub project_key: String,
    pub name: String,
    pub visibility: Visibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_role: Option<MembershipRole>,
}

#[derive(Debug, Clone)]
pub struct NamespaceMemberRecord {
    pub user_id: i64,
    pub login_name: String,
    pub display_name: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct PersonalGroupRecord {
    pub group_id: i64,
    pub group_key: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone)]
pub struct AccessScope {
    pub user_id: Option<i64>,
    pub include_public: bool,
    pub private_project_ids: Vec<i64>,
    pub group_key: Option<String>,
    pub project_key: Option<String>,
}
