use chrono::{DateTime, Utc};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
    pub project_key: String,
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
