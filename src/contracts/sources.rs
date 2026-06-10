use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use context69_contracts::sources::{
    ListSourcesResponse, SourceOriginStatusKind, SourceStatus, SyncOutcome,
};

use super::Visibility;

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
