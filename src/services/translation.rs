use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use context69_translation::{
    TranslationChunkPublication, TranslationPublication, TranslationPublisher,
};
use tracing::info;

use crate::{
    chunking::{ChunkingConfig, chunk_document},
    contracts::Visibility,
    domain::{ChunkPayload, SourceRecord},
    embedding::EmbeddingProvider,
    normalize::normalize_record,
    qdrant_index::QdrantIndex,
};

#[derive(Clone)]
pub struct TranslationPublisherAdapter {
    embedding: Option<Arc<dyn EmbeddingProvider>>,
    index: Option<QdrantIndex>,
    chunking: ChunkingConfig,
}

impl TranslationPublisherAdapter {
    pub fn new(
        embedding: Option<Arc<dyn EmbeddingProvider>>,
        index: Option<QdrantIndex>,
        chunking: ChunkingConfig,
    ) -> Self {
        Self {
            embedding,
            index,
            chunking,
        }
    }

    fn runtime(&self) -> Result<(&Arc<dyn EmbeddingProvider>, &QdrantIndex)> {
        self.embedding
            .as_ref()
            .zip(self.index.as_ref())
            .ok_or_else(|| anyhow!("translation embedding runtime is unavailable"))
    }
}

#[async_trait]
impl TranslationPublisher for TranslationPublisherAdapter {
    async fn publish(
        &self,
        old_chunk_ids: &[uuid::Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        let started = Instant::now();
        let (embedding, index) = self.runtime()?;
        let visibility = translation
            .visibility
            .parse::<Visibility>()
            .context("invalid translation document visibility")?;
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
        let payloads = chunks
            .iter()
            .map(|chunk| ChunkPayload {
                chunk_id: chunk.id,
                document_id: translation.document_id,
                group_id: translation.group_id,
                group_key: translation.group_key.to_string(),
                group_path: translation.group_path.to_string(),
                visibility,
                source_key: translation.source_key.to_string(),
                external_id: translation.external_id.to_string(),
                title: translation.title.to_string(),
                summary: translation.summary.map(ToOwned::to_owned),
                source_uri: translation.source_uri.to_string(),
                published_at: translation.published_at,
                updated_at_source: translation.updated_at,
                record_hash: normalized.record_hash.clone(),
                chunk_index: chunk.chunk_index,
                chunk_text: chunk.text.clone(),
                metadata_json: translation.metadata_json.clone(),
                content_locale: translation.target_locale.to_string(),
                source_locale: translation.source_locale.map(ToOwned::to_owned),
                translation_provider: Some(translation.provider_key.to_string()),
            })
            .collect::<Vec<_>>();
        index
            .replace_document_chunks(old_chunk_ids, &payloads, &embeddings)
            .await?;
        info!(
            document_id = translation.document_id,
            target_locale = translation.target_locale,
            batch_size = payloads.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "translation chunks published"
        );
        Ok(payloads
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
            .collect())
    }

    async fn delete(&self, chunk_ids: &[uuid::Uuid]) -> Result<()> {
        if let Some(index) = &self.index {
            index.delete_points(chunk_ids).await?;
        }
        Ok(())
    }
}
