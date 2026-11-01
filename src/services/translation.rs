use std::time::Instant;

use anyhow::Result;

use crate::domain_errors::DomainError;
use async_trait::async_trait;
use context69_translation::{
    TranslationChunkPublication, TranslationPublication, TranslationPublicationResult,
    TranslationPublisher, TranslationStore, TranslationVersionInput, publish_committed_default,
};
use tracing::info;

use crate::{
    chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, PayloadLocale, translated_chunk},
    chunking::{ChunkingConfig, chunk_document},
    contracts::Visibility,
    domain::SourceRecord,
    embedding::EmbeddingRuntime,
    normalize::normalize_record,
    qdrant_index::QdrantIndex,
    vector_runtime::{VectorRuntime, VectorSnapshot},
};

#[derive(Clone)]
pub struct TranslationPublisherAdapter {
    vector: VectorRuntime,
    chunking: ChunkingConfig,
    /// The registry-visible publication is opt-in. Without a database handle
    /// and an enabled capacity policy the adapter keeps the legacy sequence, so
    /// a deployment that has not adopted a policy is never affected.
    registry_db: Option<crate::db::Database>,
    /// The narrowing of the persisted registry capacity policy.
    ///
    /// The persisted policy is the authority: the admission reads it inside
    /// its own transaction, so this value can only lower the caps the
    /// deployment allows.
    registry_caps: Option<crate::db::RegistryQuotaOverride>,
}

impl TranslationPublisherAdapter {
    pub fn new(
        embedding: impl Into<EmbeddingRuntime>,
        index: Option<QdrantIndex>,
        chunking: ChunkingConfig,
    ) -> Self {
        Self::with_vector_runtime(
            VectorRuntime::from_legacy(embedding.into(), index, ""),
            chunking,
        )
    }

    pub fn with_vector_runtime(vector: VectorRuntime, chunking: ChunkingConfig) -> Self {
        Self {
            vector,
            chunking,
            registry_db: None,
            registry_caps: None,
        }
    }

    /// Enables the registry-visible committed publication.
    ///
    /// The quota is a narrowing of the persisted capacity policy: without a
    /// database handle the publication keeps the legacy sequence, and the
    /// admission applies the persisted caps.
    #[must_use]
    pub fn with_committed_publication(
        mut self,
        db: crate::db::Database,
        quota: crate::db::RegistryQuotaOverride,
    ) -> Self {
        self.registry_db = Some(db);
        self.registry_caps = Some(quota);
        self
    }

    fn runtime(&self) -> Result<(VectorSnapshot, Option<String>)> {
        // Translation publishes new vectors, so it waits out an
        // identity-changing rebuild instead of adding vectors to a collection
        // being re-embedded. The identity comes from the captured pair, so a
        // publication never records the identity of another configuration.
        let snapshot = self.vector.require_ready().map_err(|error| {
            anyhow::Error::from(DomainError::unavailable(format!(
                "translation embedding runtime is unavailable: {error}"
            )))
        })?;
        let fingerprint = snapshot.configured_fingerprint.clone();
        Ok((snapshot, fingerprint))
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
        let (runtime, _fingerprint) = self.runtime()?;
        let embedding = runtime.embedding.clone();
        let index = runtime.index.clone();
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

    async fn publish_committed(
        &self,
        store: &TranslationStore,
        old_chunk_ids: &[uuid::Uuid],
        translation: TranslationPublication<'_>,
        version: &TranslationVersionInput<'_>,
    ) -> Result<()> {
        let (Some(db), Some(quota)) = (self.registry_db.as_ref(), self.registry_caps) else {
            return publish_committed_default(self, store, old_chunk_ids, translation, version)
                .await;
        };
        let Some(file_id) = db
            .find_unique_library_file_for_document(translation.document_id)
            .await?
        else {
            return publish_committed_default(self, store, old_chunk_ids, translation, version)
                .await;
        };
        let (runtime, fingerprint) = self.runtime()?;
        let identity = fingerprint.map(|embedding_fingerprint| crate::db::RegistryVectorIdentity {
            embedding_fingerprint,
            collection_name: runtime.collection.clone(),
        });
        match crate::services::translation_registry::publish_translation(
            &crate::services::translation_registry::Publication {
                db,
                store,
                snapshot: &runtime,
                chunking: &self.chunking,
                quota,
                identity,
            },
            file_id,
            old_chunk_ids,
            &translation,
            version,
        )
        .await?
        {
            Some(()) => Ok(()),
            None => {
                publish_committed_default(self, store, old_chunk_ids, translation, version).await
            }
        }
    }

    async fn delete(&self, chunk_ids: &[uuid::Uuid]) -> Result<()> {
        if let Some(snapshot) = self.vector.snapshot() {
            snapshot.index.delete_points(chunk_ids).await?;
        }
        Ok(())
    }
}
