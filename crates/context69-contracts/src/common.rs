use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct Pagination {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub total_pages: u32,
}

impl Pagination {
    pub fn try_new(page: u32, page_size: u32, total: u64) -> anyhow::Result<Self> {
        if page == 0 {
            return Err(anyhow::anyhow!("page must be greater than 0"));
        }
        if !(1..=100).contains(&page_size) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        let total_pages = if total == 0 {
            0
        } else {
            u32::try_from(total.div_ceil(u64::from(page_size)))?
        };
        Ok(Self {
            page,
            page_size,
            total,
            total_pages,
        })
    }

    pub fn offset(page: u32, page_size: u32) -> anyhow::Result<i64> {
        if page == 0 {
            return Err(anyhow::anyhow!("page must be greater than 0"));
        }
        if !(1..=100).contains(&page_size) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        i64::from(page - 1)
            .checked_mul(i64::from(page_size))
            .ok_or_else(|| anyhow::anyhow!("page offset is too large"))
    }
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_processing_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_dependency_gates: Option<Vec<crate::LibraryDependencyGateResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library_processing_queue: Option<crate::LibraryProcessingQueueHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    /// Stable machine-readable error code for programmatic handling.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiErrorResponse {
    pub fn new(code: &str, message: String) -> Self {
        Self {
            code: code.to_string(),
            message,
            details: None,
        }
    }

    pub fn code_for_status(status: u16) -> &'static str {
        match status {
            400 => "invalid_argument",
            401 => "unauthorized",
            403 => "forbidden",
            404 => "not_found",
            409 => "conflict",
            413 => "payload_too_large",
            422 => "unprocessable_entity",
            429 => "rate_limited",
            502 => "upstream_error",
            503 => "unavailable",
            504 => "upstream_timeout",
            _ => "internal",
        }
    }
}
