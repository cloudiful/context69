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
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    pub error: String,
}
