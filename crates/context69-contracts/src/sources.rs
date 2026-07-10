use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Visibility;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceOriginStatusKind {
    Unknown,
    Connected,
    Unreachable,
    Misconfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SourceStatus {
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub example_queries: Vec<String>,
    pub connection: String,
    pub has_database_url: bool,
    pub origin_status: SourceOriginStatusKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_message: Option<String>,
    pub sync_strategy: String,
    pub connector_type: String,
    pub base_query: String,
    pub batch_size: i64,
    pub last_cursor_updated_at: Option<DateTime<Utc>>,
    pub last_cursor_external_id: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListSourcesResponse {
    pub sources: Vec<SourceStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SyncOutcome {
    pub records_seen: usize,
    pub records_changed: usize,
    pub chunks_upserted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SourceConfigInput {
    pub source_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub example_queries: Vec<String>,
    pub connection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_url: Option<String>,
    pub sync_strategy: String,
    pub connector_type: String,
    pub base_query: String,
    pub batch_size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SourceConnectionResponse {
    pub name: String,
    pub has_database_url: bool,
    pub origin_status: SourceOriginStatusKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertSourceConnectionRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSourceFolderRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_folder_id: Option<Uuid>,
    pub folder_name: String,
    pub source_config: SourceConfigInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SourceFolderResponse {
    pub folder_id: Uuid,
    pub source_config_file_id: Uuid,
    pub records_folder_id: Uuid,
    pub path: String,
}
