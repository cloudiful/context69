//! Service-level coverage for the same-content reuse flow: a minimal
//! `LibraryService` (no embedding runtime, no-op callbacks) drives the
//! production prepare-upload and task-upload branches. Gated on CONTEXT69_TEST_DATABASE_URL.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::contracts::{
    LibraryFileSummary, LibraryFileUploadMetadata, PrepareLibraryUploadRequest,
};
use context69::db::Database;
use context69::services::library::{LibraryService, LibraryServiceConfig, UploadedLibraryFile};
use context69::services::settings::SettingsService;
use context69_extraction::{
    ExtractionDependencies, ExtractionPublication, ExtractionPublisher, ExtractionReadiness,
    ExtractionService,
};
use context69_namespace::GroupRecord;
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

const URI_A: &str = "https://example.com/a";
const URI_B: &str = "https://example.com/b";

/// No-op stand-in for the translation/extraction callbacks LibraryService requires.
struct NoopCallbacks;

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

async fn connect_db() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

async fn build_library_service(db: &Database) -> (LibraryService, std::path::PathBuf) {
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
    let service = LibraryService::new(
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
            embedding_vector_configuration_fingerprint: "test".to_string(),
        },
        settings,
        translation,
        extraction,
    )
    .await
    .expect("build library service");
    (service, storage_root)
}

/// Deterministic payload well below the 1 MiB upload ceiling.
fn prepare_bytes() -> (Bytes, String, i64) {
    let bytes = Bytes::from((0..128u32).map(|value| value as u8).collect::<Vec<_>>());
    let size = bytes.len() as i64;
    let sha = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    (bytes, sha, size)
}
fn ts(value: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(value)
        .expect("fixed timestamp")
        .into()
}

async fn seed_group_record(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'public', 'shared', $3) RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(format!("dup-service-{}", Uuid::new_v4()))
    .bind("Duplicate Content Service Group")
    .bind(format!("test/dup-service-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    GroupRecord {
        id: row.get("id"),
        parent_group_id: None,
        group_key: row.get("group_key"),
        group_path: row.get("full_path"),
        parent_group_path: None,
        name: row.get("name"),
        visibility: context69_contracts::Visibility::Public,
        kind: context69_contracts::GroupKind::Shared,
        owner_user_id: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        current_role: None,
    }
}

async fn seed_folder(db: &Database, group_id: i64, name: &str) -> Uuid {
    let folder_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_folders (id, group_id, parent_id, name, visibility) VALUES ($1, $2, NULL, $3, 'public')",
    )
    .bind(folder_id)
    .bind(group_id)
    .bind(name)
    .execute(db.pool())
    .await
    .expect("seed folder");
    folder_id
}

async fn cleanup_group(db: &Database, group_id: i64) {
    for statement in [
        "DELETE FROM context69.library_files WHERE group_id = $1",
        "DELETE FROM context69.library_folders WHERE group_id = $1",
        "DELETE FROM context69.library_storage_objects WHERE group_id = $1",
        "DELETE FROM context69.groups WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean up test rows");
    }
}

async fn file_external_id(db: &Database, file_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT external_id FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load external_id")
}
async fn file_storage_object_id(db: &Database, file_id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT storage_object_id FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load storage_object_id")
}
/// Land the original file via the task flow so the shared storage object exists.
async fn land_original_file(
    service: &LibraryService,
    group_id: i64,
    folder_id: Uuid,
    bytes: &Bytes,
    sha256: &str,
) -> LibraryFileSummary {
    service
        .prepare_file_for_task(
            group_id,
            UploadedLibraryFile {
                folder_id: Some(folder_id),
                filename: "report.txt".to_string(),
                media_type: "text/plain".to_string(),
                bytes: bytes.clone(),
                declared_sha256: Some(sha256.to_string()),
                metadata: Some(LibraryFileUploadMetadata {
                    external_id: Some("disclosure-A".to_string()),
                    source_uri: Some("https://example.com/a".to_string()),
                    published_at: None,
                    metadata_json: serde_json::json!({"origin": "first"}),
                }),
                translation: None,
                extraction: None,
                staged_storage_object_id: None,
            },
            Uuid::new_v4(),
        )
        .await
        .expect("original upload via task flow")
}

#[tokio::test]
async fn prepare_upload_reuses_storage_and_creates_new_file_row() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping prepare-upload service test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let folder = seed_folder(&db, group.id, "reports").await;
    let other_folder = seed_folder(&db, group.id, "archive").await;
    let (bytes, sha, size) = prepare_bytes();
    let first = land_original_file(&service, group.id, folder, &bytes, &sha).await;
    let first_object = file_storage_object_id(&db, first.file_id)
        .await
        .expect("object");

    // Same SHA + size but distinct external_id: prepare-upload claims a fresh
    // row so the API handler submits the normal file task against it.
    let published_at = ts("2026-08-24T10:00:00Z");
    let prepared = service
        .prepare_upload_in_project(
            &group,
            &PrepareLibraryUploadRequest {
                folder_id: Some(other_folder),
                filename: "report.txt".to_string(),
                media_type: "text/plain".to_string(),
                size_bytes: size,
                sha256: sha.clone(),
                metadata: Some(LibraryFileUploadMetadata {
                    external_id: Some("disclosure-B".to_string()),
                    source_uri: Some("https://example.com/b".to_string()),
                    published_at: Some(published_at),
                    metadata_json: serde_json::json!({"origin": "second"}),
                }),
                translation: None,
                extraction: None,
            },
        )
        .await
        .expect("prepare-upload duplicate content");
    assert!(
        !prepared.upload_required,
        "duplicate must not require upload"
    );
    let prepared_file = prepared.file.expect("response must carry the new file");
    // Returned summaries must carry requested metadata, not stale insert values.
    assert_eq!(prepared_file.external_id.as_deref(), Some("disclosure-B"));
    assert_eq!(prepared_file.source_uri.as_deref(), Some(URI_B));
    assert_eq!(prepared_file.published_at, Some(published_at));
    assert_eq!(prepared_file.metadata_json["origin"], json!("second"));
    assert_eq!(prepared_file.filename, "report.txt");
    assert_eq!(prepared_file.folder_id, Some(other_folder));
    assert_eq!(
        file_storage_object_id(&db, prepared_file.file_id).await,
        Some(first_object)
    );

    // The original row keeps its identity and storage binding.
    assert_eq!(
        file_external_id(&db, first.file_id).await.as_deref(),
        Some("disclosure-A")
    );
    assert_eq!(
        file_storage_object_id(&db, first.file_id).await,
        Some(first_object)
    );
    assert_eq!(first.source_uri.as_deref(), Some(URI_A));
    assert_eq!(first.metadata_json["origin"], json!("first"));

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn task_upload_helper_creates_new_file_row_sharing_storage() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping task upload service test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let folder = seed_folder(&db, group.id, "reports").await;
    let other_folder = seed_folder(&db, group.id, "archive").await;
    let (bytes, sha, _) = prepare_bytes();

    let first = land_original_file(&service, group.id, folder, &bytes, &sha).await;
    let first_object = file_storage_object_id(&db, first.file_id)
        .await
        .expect("object");

    let published_at = ts("2026-08-24T11:00:00Z");
    let second = service
        .prepare_file_for_task(
            group.id,
            UploadedLibraryFile {
                folder_id: Some(other_folder),
                filename: "report.txt".to_string(),
                media_type: "text/plain".to_string(),
                bytes: bytes.clone(),
                declared_sha256: Some(sha.clone()),
                metadata: Some(LibraryFileUploadMetadata {
                    external_id: Some("disclosure-B".to_string()),
                    source_uri: Some("https://example.com/b".to_string()),
                    published_at: Some(published_at),
                    metadata_json: serde_json::json!({"origin": "second"}),
                }),
                translation: None,
                extraction: None,
                staged_storage_object_id: None,
            },
            Uuid::new_v4(),
        )
        .await
        .expect("duplicate task upload must succeed");

    assert_eq!(second.external_id.as_deref(), Some("disclosure-B"));
    assert_eq!(second.folder_id, Some(other_folder));
    assert_eq!(second.filename, "report.txt");
    // Returned summaries must carry requested metadata, not stale insert values.
    assert_eq!(second.source_uri.as_deref(), Some(URI_B));
    assert_eq!(second.published_at, Some(published_at));
    assert_eq!(second.metadata_json["origin"], json!("second"));
    assert_eq!(
        file_storage_object_id(&db, second.file_id).await,
        Some(first_object)
    );

    // Original row must be unchanged.
    assert_eq!(
        file_external_id(&db, first.file_id).await.as_deref(),
        Some("disclosure-A")
    );
    assert_eq!(
        file_storage_object_id(&db, first.file_id).await,
        Some(first_object)
    );
    assert_eq!(first.source_uri.as_deref(), Some(URI_A));
    assert_eq!(first.metadata_json["origin"], json!("first"));

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
