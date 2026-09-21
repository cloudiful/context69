//! Shared fixtures for the metadata-index paging regression tests.
//!
//! Seeding, service construction, and cleanup only; assertions stay in the
//! case files.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::contracts::{MetadataIndexResponse, MetadataIndexStatus};
use context69::db::{Database, StoredMetadataIndex};
use context69::services::document_store::DocumentStoreService;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::settings::SettingsService;
use context69_extraction::{
    ExtractionDependencies, ExtractionPublication, ExtractionPublisher, ExtractionReadiness,
    ExtractionService,
};
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use sqlx::Row;
use uuid::Uuid;

/// No-op stand-in for the translation/extraction callbacks `LibraryService`
/// requires; the paging build never reaches them.
pub struct NoopCallbacks;

#[async_trait]
impl TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

#[async_trait]
impl ExtractionPublisher for NoopCallbacks {
    async fn publish(&self, _publication: &ExtractionPublication<'_>) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ExtractionReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

pub async fn connect_db() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

pub async fn seed_group(db: &Database, prefix: &str) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("{prefix}-{}", Uuid::new_v4()))
    .bind("Metadata Paging Test Group")
    .bind(format!("test/{prefix}-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

pub async fn seed_documents(db: &Database, group_id: i64, source_key: &str, count: i64) {
    sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, \
          updated_at_source, record_hash, metadata_json, visibility) \
         SELECT $1, $2, 'paged-' || series.value, 'Paged ' || series.value, NULL, \
                'https://example.test/paged/' || series.value, now(), \
                'paging-hash-' || series.value, \
                jsonb_build_object('year', 2000 + (series.value % 10)), 'public' \
         FROM generate_series(1, $3) AS series(value)",
    )
    .bind(group_id)
    .bind(source_key)
    .bind(count)
    .execute(db.pool())
    .await
    .expect("seed test documents");
}

pub async fn document_ids(db: &Database, group_id: i64, source_key: &str) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT id FROM context69.documents \
         WHERE group_id = $1 AND source_key = $2 ORDER BY id",
    )
    .bind(group_id)
    .bind(source_key)
    .fetch_all(db.pool())
    .await
    .expect("load document ids")
}

pub fn definition(group_id: i64, source_key: &str) -> StoredMetadataIndex {
    StoredMetadataIndex {
        index_id: Uuid::new_v4(),
        group_id,
        group_path: "test/metadata-paging".to_string(),
        source_key: source_key.to_string(),
        field_path: "year".to_string(),
        data_type: "integer".to_string(),
        value_kind: "scalar".to_string(),
        sortable: true,
        status: "building".to_string(),
        processed_documents: 0,
        total_documents: 0,
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

pub async fn build_document_store_service(db: &Database) -> DocumentStoreService {
    let storage_root = std::env::temp_dir().join(format!("context69-test-{}", Uuid::new_v4()));
    let settings = SettingsService::new(db.clone());
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    let extraction = ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            file_library: FileLibraryConfig {
                storage_root: storage_root.clone(),
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "metadata-paging".to_string(),
        },
        settings,
        translation,
        extraction,
    )
    .await
    .expect("build library service");
    DocumentStoreService::new(db.clone(), None, library)
}

pub async fn wait_for_ready(
    service: &DocumentStoreService,
    group_id: i64,
    source_key: &str,
    index_id: Uuid,
) -> MetadataIndexResponse {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let indexes = service
            .list_indexes(group_id, source_key)
            .await
            .expect("list metadata indexes");
        if let Some(index) = indexes.into_iter().find(|index| index.index_id == index_id) {
            match index.status {
                MetadataIndexStatus::Ready => return index,
                MetadataIndexStatus::Failed => {
                    panic!("metadata index build failed: {:?}", index.error_message)
                }
                MetadataIndexStatus::Building | MetadataIndexStatus::Deleting => {}
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "metadata index build did not finish in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.metadata_index_definitions WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean metadata indexes");
    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean documents");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean group");
}
