//! Generation-scoped Git code vector writes (issue 25 P2).
//!
//! The code path is independent of the document vector runtime: it captures one
//! immutable code snapshot plus the runtime Qdrant endpoint, derives one
//! versioned destination, embeds the stored canonical chunks through the code
//! provider only, and acknowledges every point before the generation records its
//! metadata and activates.
//!
//! Lexical generation never enters this module. A non-lexical generation with
//! no configured code model still activates lexically without metadata; a
//! non-lexical generation whose code model is configured but whose runtime
//! Qdrant endpoint is missing fails honestly instead of silently downgrading.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::code_embedding_runtime::CodeEmbeddingSnapshot;
use crate::config::QdrantConfig;
use crate::db::{Database, MAX_GIT_LIST_PAGE};
use crate::domain_errors::DomainError;
use crate::qdrant_index::QdrantIndex;
use crate::qdrant_index::code_vectors::{CodeVectorPoint, code_collection_name};

/// One captured code destination: the immutable snapshot, endpoint, physical
/// collection, and destination identity. The endpoint is never stored in
/// metadata or logged; only its hash reaches the fingerprint.
#[derive(Clone)]
pub(crate) struct CodeVectorTarget {
    snapshot: Arc<CodeEmbeddingSnapshot>,
    qdrant_url: String,
    collection_name: String,
    destination_fingerprint: String,
    endpoint_identity: String,
}

impl CodeVectorTarget {
    fn new(snapshot: Arc<CodeEmbeddingSnapshot>, qdrant_url: &str) -> Result<Self> {
        let dimensions = snapshot.config.dimensions;
        if dimensions == 0 {
            return Err(anyhow!(DomainError::invalid_argument(
                "git_code_dimensions_invalid"
            )));
        }
        let destination_fingerprint =
            code_destination_fingerprint(&snapshot.fingerprint, qdrant_url, dimensions);
        Ok(Self {
            snapshot,
            qdrant_url: qdrant_url.trim().to_string(),
            collection_name: code_collection_name(&destination_fingerprint),
            destination_fingerprint,
            endpoint_identity: code_endpoint_identity(qdrant_url),
        })
    }

    pub(crate) fn snapshot(&self) -> &Arc<CodeEmbeddingSnapshot> {
        &self.snapshot
    }

    pub(crate) fn dimensions(&self) -> usize {
        self.snapshot.config.dimensions
    }

    pub(crate) fn qdrant_url(&self) -> &str {
        &self.qdrant_url
    }

    pub(crate) fn collection_name(&self) -> &str {
        &self.collection_name
    }

    pub(crate) fn destination_fingerprint(&self) -> &str {
        &self.destination_fingerprint
    }

    pub(crate) fn endpoint_identity(&self) -> &str {
        &self.endpoint_identity
    }

    /// Whether one stored generation metadata describes this exact destination.
    pub(crate) fn matches_metadata(
        &self,
        metadata: &crate::db::StoredGitGenerationCodeVectors,
    ) -> bool {
        metadata.fingerprint == self.destination_fingerprint
            && metadata.collection_name == self.collection_name
            && usize::try_from(metadata.dimensions)
                .map(|dimensions| dimensions == self.dimensions())
                .unwrap_or(false)
    }
}

/// The code snapshot and runtime endpoint captured once in `process_git_index`.
///
/// A test may attach its own sink through the private seam; production always
/// builds the acknowledged Qdrant sink from the captured target.
pub(crate) struct CodeVectorRequest {
    pub(crate) snapshot: Arc<CodeEmbeddingSnapshot>,
    pub(crate) qdrant_url: Option<String>,
    #[cfg(test)]
    pub(crate) sink: Option<Arc<dyn CodeVectorSink>>,
}

impl CodeVectorRequest {
    pub(crate) fn production(
        snapshot: Arc<CodeEmbeddingSnapshot>,
        qdrant_url: Option<String>,
    ) -> Self {
        Self {
            snapshot,
            qdrant_url,
            #[cfg(test)]
            sink: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_sink(
        snapshot: Arc<CodeEmbeddingSnapshot>,
        qdrant_url: Option<String>,
        sink: Arc<dyn CodeVectorSink>,
    ) -> Self {
        Self {
            snapshot,
            qdrant_url,
            sink: Some(sink),
        }
    }

    /// The injected test sink, if any. Production builds never carry one.
    pub(crate) fn test_sink(&self) -> Option<Arc<dyn CodeVectorSink>> {
        #[cfg(test)]
        {
            self.sink.clone()
        }
        #[cfg(not(test))]
        {
            None
        }
    }
}

/// Resolves the immutable destination of one code request.
///
/// A configured code model with no runtime Qdrant endpoint is a configuration
/// error, not a silent lexical downgrade.
pub(crate) fn code_vector_target(request: &CodeVectorRequest) -> Result<CodeVectorTarget> {
    let url = request
        .qdrant_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| anyhow!(DomainError::invalid_argument("git_code_qdrant_url_missing")))?;
    CodeVectorTarget::new(Arc::clone(&request.snapshot), url)
}

/// The deterministic identity of one isolated code destination.
///
/// It composes the code vector-space fingerprint with the normalized Qdrant
/// endpoint and the dimensions. The framing is length-prefixed, so no two
/// distinct inputs can hash the same concatenation. Credentials, the document
/// collection name, and the document identity never participate.
pub(crate) fn code_destination_fingerprint(
    snapshot_fingerprint: &str,
    qdrant_url: &str,
    dimensions: usize,
) -> String {
    let normalized = normalize_qdrant_endpoint(qdrant_url);
    let parts = [
        snapshot_fingerprint.trim().to_string(),
        normalized,
        dimensions.to_string(),
    ];
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Normalizes a Qdrant endpoint for identity only.
///
/// Whitespace and trailing slashes are not a different destination. The value is
/// hashed, never persisted as the raw endpoint.
pub(crate) fn normalize_qdrant_endpoint(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

/// The destination identity of one normalized Qdrant endpoint.
///
/// Only the endpoint participates, so cleanup can decide whether the points it
/// is about to remove belong to the currently configured destination. The
/// embedding model fingerprint and physical collection are deliberately
/// excluded: a model change does not move the points, and the collection is
/// already stored beside this identity. The raw endpoint and any credentials
/// are never persisted.
pub(crate) fn code_endpoint_identity(value: &str) -> String {
    let normalized = normalize_qdrant_endpoint(value);
    let mut digest = Sha256::new();
    digest.update((normalized.len() as u64).to_le_bytes());
    digest.update(normalized.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A narrow write seam so orchestration tests can observe count and failure
/// without a live Qdrant server. A successful fake is never evidence of remote
/// durability; only the acknowledged production sink is.
#[async_trait]
pub(crate) trait CodeVectorSink: Send + Sync {
    async fn write(&self, dimensions: usize, points: &[CodeVectorPoint]) -> Result<()>;

    /// Reads stored vectors for the exact chunk ids a reuse pass wants.
    ///
    /// The default keeps narrow write-only sinks valid; a sink that can serve
    /// a reuse pass returns the vectors it actually stores.
    async fn retrieve(&self, _point_ids: &[Uuid]) -> Result<Vec<(Uuid, Vec<f32>)>> {
        Ok(Vec::new())
    }
}

/// The production sink: an insert-only, acknowledged write to the captured
/// code collection.
pub(crate) struct QdrantCodeVectorSink {
    index: QdrantIndex,
}

impl QdrantCodeVectorSink {
    pub(crate) async fn open(target: &CodeVectorTarget) -> Result<Self> {
        let config = QdrantConfig {
            url: target.qdrant_url().to_string(),
            collection_name: target.collection_name().to_string(),
            recreate_on_dimension_mismatch: false,
        };
        let index = QdrantIndex::ensure_code_collection(
            &config,
            target.collection_name(),
            target.dimensions(),
        )
        .await?;
        Ok(Self { index })
    }
}

#[async_trait]
impl CodeVectorSink for QdrantCodeVectorSink {
    async fn write(&self, _dimensions: usize, points: &[CodeVectorPoint]) -> Result<()> {
        self.index.upsert_code_points(points).await
    }

    async fn retrieve(&self, point_ids: &[Uuid]) -> Result<Vec<(Uuid, Vec<f32>)>> {
        self.index.retrieve_code_vectors(point_ids).await
    }
}

/// Embeds every stored chunk of one building generation and acknowledges it
/// through `sink`, returning the acknowledged point count.
///
/// Files and chunks are paged with the database listing bound, and the provider
/// batches each bounded page itself, so no whole-generation text or vector list
/// is ever held in memory. The returned vector count and every dimension are
/// checked before a point reaches the sink.
pub(crate) async fn embed_and_write_generation(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
    generation_key: Uuid,
    target: &CodeVectorTarget,
    sink: &dyn CodeVectorSink,
) -> Result<i64> {
    let dimensions = target.dimensions();
    let mut acknowledged = 0i64;
    let mut file_offset = 0i64;
    loop {
        let files = db
            .list_git_generation_files(
                group_id,
                repository_key,
                generation_key,
                MAX_GIT_LIST_PAGE,
                file_offset,
            )
            .await?;
        if files.is_empty() {
            break;
        }
        for file in &files {
            let mut chunk_offset = 0i64;
            loop {
                let chunks = db
                    .list_git_generation_chunks(
                        group_id,
                        repository_key,
                        generation_key,
                        file.file_key,
                        MAX_GIT_LIST_PAGE,
                        chunk_offset,
                    )
                    .await?;
                if chunks.is_empty() {
                    break;
                }
                let texts = chunks
                    .iter()
                    .map(|chunk| chunk.text.clone())
                    .collect::<Vec<_>>();
                let vectors = target.snapshot().provider.embed_texts(&texts).await?;
                validate_embedding_vectors(&vectors, texts.len(), dimensions)?;
                let points = chunks
                    .iter()
                    .zip(vectors)
                    .map(|(chunk, vector)| CodeVectorPoint {
                        chunk_key: chunk.chunk_key,
                        vector,
                        group_id,
                        repository_key,
                        generation_key,
                        file_key: file.file_key,
                        chunk_index: chunk.chunk_index,
                        path: file.path.clone(),
                        language: file.language.clone(),
                        start_line: chunk.start_line,
                        end_line: chunk.end_line,
                    })
                    .collect::<Vec<_>>();
                sink.write(dimensions, &points).await?;
                acknowledged = acknowledged.saturating_add(bounded_count(points.len()));
                if (chunks.len() as i64) < MAX_GIT_LIST_PAGE {
                    break;
                }
                chunk_offset += chunks.len() as i64;
            }
        }
        if (files.len() as i64) < MAX_GIT_LIST_PAGE {
            break;
        }
        file_offset += files.len() as i64;
    }
    Ok(acknowledged)
}

fn bounded_count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Rejects a provider page before any point reaches the sink.
///
/// The count, every dimension, and finiteness are checked together so a
/// malformed provider page fails the generation before metadata is recorded or
/// activation is attempted.
pub(crate) fn validate_embedding_vectors(
    vectors: &[Vec<f32>],
    expected_count: usize,
    dimensions: usize,
) -> Result<()> {
    if vectors.len() != expected_count {
        return Err(anyhow!(DomainError::upstream_error(
            "git_code_embedding_count_mismatch"
        )));
    }
    for vector in vectors {
        if vector.len() != dimensions {
            return Err(anyhow!(DomainError::upstream_error(
                "git_code_embedding_dimension_mismatch"
            )));
        }
        if !vector.iter().all(|value| value.is_finite()) {
            return Err(anyhow!(DomainError::upstream_error(
                "git_code_embedding_non_finite"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "code_vectors_tests.rs"]
mod tests;
