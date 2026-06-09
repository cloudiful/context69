use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_chunks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qdrant_ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    pub error: String,
}
