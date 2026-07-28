mod providers;
mod segmenter;
mod store;
mod worker;

use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts::{TranslationDirective, TranslationJobResponse};
use sqlx::PgPool;
use uuid::Uuid;

pub use store::{StoredGroupTranslationSettings, TranslationStore};
pub use worker::TranslationService;

#[derive(Debug, Clone)]
pub struct TranslationChunkPublication {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub target_locale: String,
    pub source_locale: Option<String>,
    pub provider_key: String,
    pub chunk_index: i32,
    pub chunk_text: String,
}

pub type TranslationRollback = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

pub struct TranslationPublicationResult {
    pub chunks: Vec<TranslationChunkPublication>,
    rollback: Option<TranslationRollback>,
}

impl TranslationPublicationResult {
    pub fn completed(chunks: Vec<TranslationChunkPublication>) -> Self {
        Self {
            chunks,
            rollback: None,
        }
    }

    pub fn with_rollback(
        chunks: Vec<TranslationChunkPublication>,
        rollback: TranslationRollback,
    ) -> Self {
        Self {
            chunks,
            rollback: Some(rollback),
        }
    }

    pub async fn rollback(self) -> Result<()> {
        match self.rollback {
            Some(rollback) => rollback.await,
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranslationPublication<'a> {
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
    pub target_locale: &'a str,
    pub source_locale: Option<&'a str>,
    pub provider_key: &'a str,
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub body_text: &'a str,
}

#[async_trait]
pub trait TranslationPublisher: Send + Sync {
    async fn publish(
        &self,
        old_chunk_ids: &[Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>>;

    async fn publish_with_rollback(
        &self,
        old_chunk_ids: &[Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<TranslationPublicationResult> {
        Ok(TranslationPublicationResult::completed(
            self.publish(old_chunk_ids, translation).await?,
        ))
    }

    async fn delete(&self, chunk_ids: &[Uuid]) -> Result<()>;
}

#[async_trait]
pub trait TranslationReadiness: Send + Sync {
    async fn is_ready(&self) -> Result<bool>;

    async fn report_processing_error(&self, _error: &str) -> Result<bool> {
        Ok(false)
    }
}

#[derive(Clone)]
pub struct TranslationDependencies {
    pub pool: PgPool,
    pub http_client: reqwest::Client,
    pub publisher: Arc<dyn TranslationPublisher>,
    pub concurrency: usize,
    pub readiness: Arc<dyn TranslationReadiness>,
}

#[derive(Debug, Clone)]
pub struct EnqueueTranslation {
    pub document_id: i64,
    pub directive: Option<TranslationDirective>,
}

#[async_trait]
pub trait TranslationCoordinator: Send + Sync {
    async fn enqueue(&self, input: EnqueueTranslation) -> Result<Vec<TranslationJobResponse>>;
}
