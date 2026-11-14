pub mod entity;
mod providers;
mod segmenter;
mod store;
mod worker;

use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
pub use context69_contracts_translation::TranslationDirective;
use context69_contracts_translation::TranslationJobResponse;
use sqlx::PgPool;
use uuid::Uuid;

pub use store::{StoredGroupTranslationSettings, TranslationStore, TranslationVersionInput};
pub use worker::TranslationService;

pub use context69_contracts_translation::{
    TranslationChunkingBudget, TranslationChunkingMode, TranslationChunkingTokenizer,
};

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
    /// The chunk budget this publication must use.
    ///
    /// `Some` is an explicit snapshot pinned by the caller (a task item), and
    /// it always wins over the live budget. `None` keeps the historical
    /// behavior of chunking with whatever the publisher currently considers
    /// active.
    pub chunking: Option<&'a TranslationChunkingBudget>,
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

    /// Publishes the points and then commits the translation version.
    ///
    /// The default sequence is the historical one: publish the points with a
    /// rollback handle, write the version row and its chunk rows, and roll the
    /// published points back when the version write fails. A publisher that can
    /// commit the points, the version and its own visibility change atomically
    /// overrides this hook instead; every existing implementor keeps the
    /// default behaviour without an edit.
    async fn publish_committed(
        &self,
        store: &TranslationStore,
        old_chunk_ids: &[Uuid],
        translation: TranslationPublication<'_>,
        version: &TranslationVersionInput<'_>,
    ) -> Result<()> {
        publish_committed_default(self, store, old_chunk_ids, translation, version).await
    }

    async fn delete(&self, chunk_ids: &[Uuid]) -> Result<()>;

    /// The budget this publisher considers active right now.
    ///
    /// An operation boundary asks for it once and passes the owned value to
    /// every publication of that operation, so a settings save cannot change
    /// the budget between locales. The default keeps every existing
    /// implementor source-compatible: without an answer, the historical
    /// per-publication resolution stays in effect.
    fn current_chunking(&self) -> Option<TranslationChunkingBudget> {
        None
    }
}

/// The default [`TranslationPublisher::publish_committed`] sequence.
///
/// Exposed so an overriding publisher can delegate to exactly the historical
/// behaviour instead of duplicating it.
pub async fn publish_committed_default<P>(
    publisher: &P,
    store: &TranslationStore,
    old_chunk_ids: &[Uuid],
    translation: TranslationPublication<'_>,
    version: &TranslationVersionInput<'_>,
) -> Result<()>
where
    P: TranslationPublisher + ?Sized,
{
    let published = publisher
        .publish_with_rollback(old_chunk_ids, translation)
        .await?;
    let chunks = published
        .chunks
        .iter()
        .map(|chunk| (chunk.chunk_id, chunk.chunk_text.clone()))
        .collect::<Vec<_>>();
    if let Err(error) = store.publish_version(version, &chunks).await {
        return match published.rollback().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(anyhow::anyhow!(
                "{error:#}; failed to roll back published translation points: {rollback_error:#}"
            )),
        };
    }
    Ok(())
}

#[async_trait]
pub trait TranslationReadiness: Send + Sync {
    async fn is_ready(&self) -> Result<bool>;

    async fn reserve_probe(&self) -> Result<Option<Uuid>> {
        Ok(None)
    }

    async fn is_ready_for_probe(&self, _probe_token: Uuid) -> Result<bool> {
        self.is_ready().await
    }

    async fn report_processing_error(&self, _error: &str) -> Result<bool> {
        Ok(false)
    }

    async fn report_processing_error_with_probe(
        &self,
        error: &str,
        _probe_token: Option<Uuid>,
    ) -> Result<bool> {
        self.report_processing_error(error).await
    }

    async fn complete_probe(&self, _probe_token: Uuid) -> Result<()> {
        Ok(())
    }

    async fn abandon_probe(&self, _probe_token: Uuid) -> Result<()> {
        Ok(())
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
    /// Blocking conversion: reuse or insert the document's translation jobs and
    /// run them to completion in the caller. There is no background worker and
    /// no dispatcher re-claim.
    async fn convert(&self, input: EnqueueTranslation) -> Result<Vec<TranslationJobResponse>>;

    /// Compatibility alias for callers that only submit work; the submission
    /// now completes inline through [`convert`](Self::convert).
    async fn enqueue(&self, input: EnqueueTranslation) -> Result<Vec<TranslationJobResponse>> {
        self.convert(input).await
    }
}
