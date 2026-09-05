pub mod providers;
mod store;
mod worker;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts::{ExtractionDirective, ExtractionJobResponse};
use sqlx::PgPool;

pub use store::{ExtractionStore, StoredExtractionTemplate};
pub use worker::ExtractionService;

#[derive(Debug, Clone)]
pub struct ExtractionPublication<'a> {
    pub document_id: i64,
    pub group_id: i64,
    pub group_key: &'a str,
    pub group_path: &'a str,
    pub visibility: &'a str,
    pub source_key: &'a str,
    pub external_id: &'a str,
    pub source_uri: &'a str,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub metadata_json: &'a serde_json::Value,
    pub source_record_hash: &'a str,
    pub template_key: &'a str,
    pub result_json: &'a serde_json::Value,
}

#[async_trait]
pub trait ExtractionPublisher: Send + Sync {
    async fn publish(&self, publication: &ExtractionPublication<'_>) -> Result<()>;
}

#[async_trait]
pub trait ExtractionReadiness: Send + Sync {
    async fn is_ready(&self) -> Result<bool>;
}

#[derive(Clone)]
pub struct ExtractionDependencies {
    pub pool: PgPool,
    pub http_client: reqwest::Client,
    pub publisher: Arc<dyn ExtractionPublisher>,
    pub concurrency: usize,
    pub readiness: Arc<dyn ExtractionReadiness>,
}

#[derive(Debug, Clone)]
pub struct EnqueueExtraction {
    pub document_id: i64,
    pub directive: ExtractionDirective,
}

#[async_trait]
pub trait ExtractionCoordinator: Send + Sync {
    async fn enqueue(&self, input: EnqueueExtraction) -> Result<Vec<ExtractionJobResponse>>;
}
