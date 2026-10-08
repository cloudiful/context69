use std::{sync::Arc, time::Instant};

use anyhow::Result;

use crate::domain_errors::DomainError;
use async_trait::async_trait;
use context69_translation::{
    TranslationChunkPublication, TranslationPublication, TranslationPublicationResult,
    TranslationPublisher,
};
use tracing::info;

use crate::{
    chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, PayloadLocale, translated_chunk},
    chunking::{ChunkingConfig, chunk_document},
    contracts::Visibility,
    domain::SourceRecord,
    embedding::{EmbeddingProvider, EmbeddingRuntime},
    normalize::normalize_record,
    qdrant_index::QdrantIndex,
};

#[derive(Clone)]
pub struct TranslationPublisherAdapter {
    embedding: EmbeddingRuntime,
    index: Option<QdrantIndex>,
    chunking: ChunkingConfig,
}

impl TranslationPublisherAdapter {
    pub fn new(
        embedding: impl Into<EmbeddingRuntime>,
        index: Option<QdrantIndex>,
        chunking: ChunkingConfig,
    ) -> Self {
        Self {
            embedding: embedding.into(),
            index,
            chunking,
        }
    }

    fn runtime(&self) -> Result<(Arc<dyn EmbeddingProvider>, &QdrantIndex)> {
        // Translation publishes new vectors, so it waits out an
        // identity-changing rebuild instead of adding vectors to a collection
        // being re-embedded.
        let embedding = self.embedding.require_ready().map_err(|error| {
            DomainError::unavailable(format!(
                "translation embedding runtime is unavailable: {error}"
            ))
        })?;
        let index = self.index.as_ref().ok_or_else(|| {
            DomainError::unavailable("translation embedding runtime is unavailable")
        })?;
        Ok((embedding, index))
    }
}

#[async_trait]
impl TranslationPublisher for TranslationPublisherAdapter {
    async fn publish(
        &self,
        old_chunk_ids: &[uuid::Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        Ok(self
            .publish_with_rollback(old_chunk_ids, translation)
            .await?
            .chunks)
    }

    async fn publish_with_rollback(
        &self,
        old_chunk_ids: &[uuid::Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<TranslationPublicationResult> {
        let started = Instant::now();
        let (embedding, index) = self.runtime()?;
        let visibility = translation
            .visibility
            .parse::<Visibility>()
            .map_err(|error| {
                DomainError::invalid_argument(format!(
                    "invalid translation document visibility: {error}"
                ))
            })?;
        let normalized = normalize_record(SourceRecord {
            external_id: translation.external_id.to_string(),
            title: translation.title.to_string(),
            summary: translation.summary.map(ToOwned::to_owned),
            body_text: translation.body_text.to_string(),
            source_uri: translation.source_uri.to_string(),
            published_at: translation.published_at,
            updated_at: translation.updated_at,
            metadata_json: translation.metadata_json.clone(),
        });
        let chunk_source_key = format!(
            "translation:{}:{}",
            translation.source_key, translation.target_locale
        );
        let chunks = chunk_document(
            translation.document_id,
            &chunk_source_key,
            &normalized,
            &self.chunking,
        );
        let texts = chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let embeddings = embedding.embed_texts(&texts).await?;
        let group = PayloadGroup {
            group_id: translation.group_id,
            group_key: translation.group_key,
            group_path: translation.group_path,
            visibility,
        };
        let document = PayloadDocument {
            source_key: translation.source_key,
            external_id: translation.external_id,
            title: translation.title,
            summary: translation.summary,
            source_uri: translation.source_uri,
            published_at: translation.published_at,
            updated_at_source: translation.updated_at,
            record_hash: &normalized.record_hash,
            metadata_json: translation.metadata_json,
        };
        let locale = PayloadLocale {
            content_locale: translation.target_locale,
            source_locale: translation.source_locale,
            translation_provider: Some(translation.provider_key),
        };
        let payloads = chunks
            .iter()
            .map(|chunk| {
                translated_chunk(
                    &group,
                    &document,
                    ChunkRef {
                        chunk_id: chunk.id,
                        document_id: translation.document_id,
                        chunk_index: chunk.chunk_index,
                        chunk_text: &chunk.text,
                    },
                    locale,
                )
            })
            .collect::<Vec<_>>();
        let replacement = index
            .replace_document_chunks_with_rollback(old_chunk_ids, &payloads, &embeddings)
            .await?;
        info!(
            document_id = translation.document_id,
            target_locale = translation.target_locale,
            batch_size = payloads.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "translation chunks published"
        );
        let chunks = payloads
            .into_iter()
            .map(|payload| TranslationChunkPublication {
                chunk_id: payload.chunk_id,
                document_id: payload.document_id,
                target_locale: payload.content_locale,
                source_locale: payload.source_locale,
                provider_key: payload.translation_provider.unwrap_or_default(),
                chunk_index: payload.chunk_index,
                chunk_text: payload.chunk_text,
            })
            .collect();
        Ok(TranslationPublicationResult::with_rollback(
            chunks,
            Box::pin(async move { replacement.rollback().await }),
        ))
    }

    async fn delete(&self, chunk_ids: &[uuid::Uuid]) -> Result<()> {
        if let Some(index) = &self.index {
            index.delete_points(chunk_ids).await?;
        }
        Ok(())
    }
}
