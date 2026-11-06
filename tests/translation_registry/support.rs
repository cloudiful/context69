//! Fixtures of the registry-visible translation publication tests.
//!
//! One fixture reuses the shared switch harness for the library file, the
//! document and its file link, plus the in-process Points mock. Embedding is a
//! local fake, so no real vector service is involved.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use context69::chunking::ChunkingConfig;
use context69::db::{Database, RegistryQuotaOverride};
use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
use context69::qdrant_index::QdrantIndex;
use context69::services::translation::TranslationPublisherAdapter;
use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
use context69_translation::{
    TranslationChunkingBudget, TranslationChunkingMode, TranslationChunkingTokenizer,
    TranslationDependencies, TranslationPublication, TranslationPublisher, TranslationReadiness,
    TranslationService, TranslationStore, TranslationVersionInput,
};

use super::harness;
use super::qdrant_points_mock::QdrantPointsMock;

pub const DIMENSIONS: usize = 4;

pub fn chunking() -> ChunkingConfig {
    ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 0,
    }
}

/// A fake embedder with one deterministic vector per text and an optional
/// failure on the first N calls.
pub struct FakeEmbedding {
    fail_first_calls: AtomicUsize,
}

impl FakeEmbedding {
    pub fn new(fail_first_calls: usize) -> Self {
        Self {
            fail_first_calls: AtomicUsize::new(fail_first_calls),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let failing =
            self.fail_first_calls
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    if remaining == 0 {
                        None
                    } else {
                        Some(remaining - 1)
                    }
                });
        if failing.is_ok() {
            return Err(anyhow::anyhow!("transient embedding outage"));
        }
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32; DIMENSIONS])
            .collect())
    }
}

/// A ready vector runtime whose index points at the in-process mock.
pub fn runtime(
    embedding: Arc<FakeEmbedding>,
    mock: &QdrantPointsMock,
    collection: &str,
) -> Result<VectorRuntime> {
    let index = QdrantIndex::for_test_unreachable(mock.url(), collection, DIMENSIONS)?;
    let provider: Arc<dyn EmbeddingProvider> = embedding.clone();
    let runtime = VectorRuntime::from_legacy(
        EmbeddingRuntime::new(Some(provider.clone()), None),
        None,
        "",
    );
    let generation = runtime.generation();
    assert!(runtime.publish(
        generation,
        VectorSnapshot {
            embedding: provider,
            identity: None,
            index: Arc::new(index),
            collection: collection.into(),
            generation,
            configured_fingerprint: Some("support-fingerprint".to_string()),
        }
    ));
    runtime.set_configured_fingerprint(Some("translation-fingerprint".to_string()));
    Ok(runtime)
}

pub fn adapter(
    runtime: VectorRuntime,
    db: Option<Database>,
    caps: Option<[i64; 3]>,
) -> TranslationPublisherAdapter {
    let adapter = TranslationPublisherAdapter::with_vector_runtime(runtime, chunking());
    match (db, caps) {
        (Some(db), Some(caps)) => {
            adapter.with_committed_publication(db, RegistryQuotaOverride::exact(caps))
        }
        _ => adapter,
    }
}

/// Publishes one translated version for a seeded document.
pub async fn publish(
    adapter: &TranslationPublisherAdapter,
    store: &TranslationStore,
    fixture: &harness::SwitchFixture,
    seeded: &harness::SeededSection,
    source_record_hash: &str,
    body: &str,
    old_chunk_ids: &[Uuid],
) -> (Uuid, Result<()>) {
    let group_key = format!("switch-{}", fixture.group);
    let group_path = format!("test/switch-{}", fixture.group);
    let source_uri = "context69://test".to_string();
    let title = "translated".to_string();
    let metadata = serde_json::json!({});
    let updated = chrono::Utc::now();
    let target_locale = "zh".to_string();
    let source_locale = "en".to_string();
    let provider_key = "llm".to_string();
    let provider_config_hash = "cfg".to_string();
    let version_id = Uuid::new_v4();
    let version = TranslationVersionInput {
        id: version_id,
        document_id: seeded.document_id,
        target_locale: &target_locale,
        source_locale: Some(&source_locale),
        source_record_hash,
        provider_key: &provider_key,
        provider_config_hash: &provider_config_hash,
        model_name: None,
        title: &title,
        summary: None,
        body_text: body,
    };
    let result = adapter
        .publish_committed(
            store,
            old_chunk_ids,
            TranslationPublication {
                document_id: seeded.document_id,
                group_id: fixture.group,
                group_key: &group_key,
                group_path: &group_path,
                visibility: "private",
                source_key: "library",
                external_id: &seeded.external_id,
                source_uri: &source_uri,
                published_at: None,
                updated_at: updated,
                metadata_json: &metadata,
                source_record_hash,
                target_locale: &target_locale,
                source_locale: Some(&source_locale),
                provider_key: &provider_key,
                title: &title,
                summary: None,
                body_text: body,
                chunking: None,
            },
            &version,
        )
        .await;
    (version_id, result)
}

/// Seeds the current translation version of a document with explicit chunk ids.
pub async fn seed_current_translation(
    store: &TranslationStore,
    seeded: &harness::SeededSection,
    chunks: &[(Uuid, String)],
) {
    let target_locale = "zh".to_string();
    let source_locale = "en".to_string();
    let provider_key = "llm".to_string();
    let provider_config_hash = "cfg".to_string();
    let title = "previous".to_string();
    store
        .publish_version(
            &TranslationVersionInput {
                id: Uuid::new_v4(),
                document_id: seeded.document_id,
                target_locale: &target_locale,
                source_locale: Some(&source_locale),
                source_record_hash: &seeded.record_hash,
                provider_key: &provider_key,
                provider_config_hash: &provider_config_hash,
                model_name: None,
                title: &title,
                summary: None,
                body_text: "previous body",
            },
            chunks,
        )
        .await
        .expect("the current translation version must seed");
}

/// A stored translation version with an explicit source hash.
///
/// The store publishes only a version of the current source hash, so a stale
/// version of an older hash is seeded directly: it stays stored beside the
/// current one and its point ids are part of the set a publication replaces.
pub async fn seed_translation_version(
    db: &context69::db::Database,
    document_id: i64,
    target_locale: &str,
    source_record_hash: &str,
    chunks: &[(Uuid, String)],
) -> Vec<Uuid> {
    let version_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.document_translation_versions \
         (id, document_id, target_locale, source_record_hash, provider_key, \
          provider_config_hash, translated_title, translated_body_text) \
         VALUES ($1, $2, $3, $4, 'test', 'test', 'test', 'test')",
    )
    .bind(version_id)
    .bind(document_id)
    .bind(target_locale)
    .bind(source_record_hash)
    .execute(db.pool())
    .await
    .expect("seed translation version");
    for (index, (chunk_id, text)) in chunks.iter().enumerate() {
        sqlx::query(
            "INSERT INTO context69.document_translation_chunks \
             (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(chunk_id)
        .bind(version_id)
        .bind(document_id)
        .bind(target_locale)
        .bind(i32::try_from(index).expect("chunk index"))
        .bind(text)
        .execute(db.pool())
        .await
        .expect("seed translation chunk");
    }
    chunks.iter().map(|(chunk_id, _)| *chunk_id).collect()
}

/// Counts how often a publisher published before the version write.
#[derive(Default)]
pub struct CountingPublisher {
    pub publishes: AtomicUsize,
}

#[async_trait]
impl TranslationPublisher for CountingPublisher {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<Vec<context69_translation::TranslationChunkPublication>> {
        self.publishes.fetch_add(1, Ordering::SeqCst);
        Ok(vec![context69_translation::TranslationChunkPublication {
            chunk_id: Uuid::new_v4(),
            document_id: translation.document_id,
            target_locale: translation.target_locale.to_string(),
            source_locale: translation.source_locale.map(ToOwned::to_owned),
            provider_key: translation.provider_key.to_string(),
            chunk_index: 0,
            chunk_text: translation.body_text.to_string(),
        }])
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}

/// A translation readiness gate that is always ready.
pub struct AlwaysReady;

#[async_trait]
impl TranslationReadiness for AlwaysReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(true)
    }
}

/// A character budget in the translated form the publisher reports.
pub fn pinned_budget(max_chars: usize, overlap_chars: usize) -> TranslationChunkingBudget {
    TranslationChunkingBudget {
        mode: TranslationChunkingMode::Chars,
        max_chars,
        overlap_chars,
        tokenizer: TranslationChunkingTokenizer::Cl100kBase,
        max_tokens: 0,
        overlap_tokens: 0,
    }
}

/// The provider row these two operation-budget cases share, so they do not
/// race each other while enabling the local endpoint.
pub static PROVIDER_CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A publisher that counts live-budget requests and records every publication's
/// actual chunk budget.
pub struct CountingSnapshotPublisher {
    pub snapshots: AtomicUsize,
    pub publications: std::sync::Mutex<Vec<Option<TranslationChunkingBudget>>>,
    budget: TranslationChunkingBudget,
}

impl CountingSnapshotPublisher {
    pub fn new() -> Self {
        Self {
            snapshots: AtomicUsize::new(0),
            publications: std::sync::Mutex::new(Vec::new()),
            budget: pinned_budget(640, 64),
        }
    }

    pub fn published_budgets(&self) -> Vec<Option<TranslationChunkingBudget>> {
        self.publications.lock().expect("publication log").clone()
    }
}

#[async_trait]
impl TranslationPublisher for CountingSnapshotPublisher {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        translation: TranslationPublication<'_>,
    ) -> Result<Vec<context69_translation::TranslationChunkPublication>> {
        self.publications
            .lock()
            .expect("publication log")
            .push(translation.chunking.cloned());
        Ok(vec![context69_translation::TranslationChunkPublication {
            chunk_id: Uuid::new_v4(),
            document_id: translation.document_id,
            target_locale: translation.target_locale.to_string(),
            source_locale: translation.source_locale.map(ToOwned::to_owned),
            provider_key: translation.provider_key.to_string(),
            chunk_index: 0,
            chunk_text: translation.body_text.to_string(),
        }])
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }

    fn current_chunking(&self) -> Option<TranslationChunkingBudget> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        Some(self.budget.clone())
    }
}

/// A translation service on the fixture database with the given publisher.
pub fn translation_service(
    db: &Database,
    publisher: Arc<dyn TranslationPublisher>,
) -> TranslationService {
    TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher,
        concurrency: 1,
        readiness: Arc::new(AlwaysReady),
    })
}

/// A loopback LibreTranslate endpoint that answers every segment.
///
/// The retained operation-budget cases need a publication to observe the
/// budget that actually reached the publisher, so this serves the smallest
/// valid provider response instead of letting the job fail before publishing.
pub struct LocalTranslationServer {
    endpoint: String,
    task: tokio::task::JoinHandle<()>,
}

impl LocalTranslationServer {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the translation stub");
        let address = listener.local_addr().expect("stub address");
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buffer = vec![0u8; 64 * 1024];
                    let Ok(read) = stream.read(&mut buffer).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let body = request
                        .find("\r\n\r\n")
                        .map(|index| &request[index + 4..])
                        .unwrap_or_default();
                    let payload: serde_json::Value =
                        serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
                    let count = payload
                        .get("q")
                        .and_then(|value| value.as_array())
                        .map(Vec::len)
                        .unwrap_or(1);
                    let translated = vec!["translated"; count];
                    let body = serde_json::json!({ "translatedText": translated }).to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            endpoint: format!("http://{address}"),
            task,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for LocalTranslationServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Points the enabled LibreTranslate provider at the local stub.
pub async fn enable_local_translation(db: &Database, endpoint: &str) {
    sqlx::query(
        "UPDATE context69.translation_provider_settings SET enabled = FALSE \
         WHERE provider_key <> 'libretranslate'",
    )
    .execute(db.pool())
    .await
    .expect("disable the other translation providers");
    sqlx::query(
        "INSERT INTO context69.translation_provider_settings \
         (provider_key, enabled, priority, endpoint, monthly_character_limit) \
         VALUES ('libretranslate', TRUE, 30, $1, 1000000) \
         ON CONFLICT (provider_key) DO UPDATE SET enabled = TRUE, \
         endpoint = EXCLUDED.endpoint, monthly_character_limit = 1000000",
    )
    .bind(endpoint)
    .execute(db.pool())
    .await
    .expect("enable the local translation provider");
}

/// Restores the provider rows to their disabled, endpoint-less state.
pub async fn disable_local_translation(db: &Database) {
    sqlx::query(
        "UPDATE context69.translation_provider_settings \
         SET enabled = FALSE, endpoint = NULL WHERE provider_key = 'libretranslate'",
    )
    .execute(db.pool())
    .await
    .expect("disable the local translation provider");
}
