//! Regression coverage for named/library text writes going through the
//! content-addressed storage object flow (`objects/{group_id}/{sha256}`):
//!
//! - new text writes must link `library_files.storage_object_id` and set
//!   `storage_rel_path` to the referenced `object_key`,
//! - identical content must reuse the same storage object,
//! - replacing a text file must keep the old content until the reference
//!   update succeeds and then release the old object,
//! - legacy UUID direct-path rows (`storage_object_id IS NULL`) must stay
//!   readable and deletable until migration completes.
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch
//! database; they are skipped otherwise.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::contracts::{LibraryTextContentFormat, UpsertLibraryTextRequest};
use context69::db::Database;
use context69::services::library::{LibraryService, LibraryServiceConfig};
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

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn seed_group_record(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'public', 'shared', $3) RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(format!("text-store-{}", Uuid::new_v4()))
    .bind("Text Storage Test Group")
    .bind(format!("test/text-store-{}", Uuid::new_v4()))
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

struct FileStorageRow {
    storage_rel_path: String,
    storage_object_id: Option<Uuid>,
}

async fn file_storage_row(db: &Database, file_id: Uuid) -> FileStorageRow {
    let row = sqlx::query(
        "SELECT storage_rel_path, storage_object_id FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load file storage columns");
    FileStorageRow {
        storage_rel_path: row.get("storage_rel_path"),
        storage_object_id: row.get("storage_object_id"),
    }
}

async fn storage_object_count(db: &Database, object_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM context69.library_storage_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("count storage object rows")
}

fn text_request(external_id: &str, title: &str, content: &str) -> UpsertLibraryTextRequest {
    UpsertLibraryTextRequest {
        external_id: external_id.to_string(),
        folder_id: None,
        title: title.to_string(),
        content: content.to_string(),
        content_format: LibraryTextContentFormat::PlainText,
        source_uri: None,
        summary: None,
        published_at: None,
        metadata_json: json!({}),
        translation: None,
        extraction: None,
    }
}

#[tokio::test]
async fn text_writes_link_content_addressed_storage_and_reuse_identical_bytes() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping text storage test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    // The service trims request content before hashing and storing bytes.
    let content = "shared alpha body\n";
    let stored = Bytes::from(content.trim());
    let sha = sha256_hex(&stored);
    let expected_key = format!("objects/{}/{}", group.id, sha);

    let first = service
        .upsert_text_file_in_project(&group, &text_request("tw-A", "Alpha", content))
        .await
        .expect("first text upsert");

    let first_row = file_storage_row(&db, first.file_id).await;
    let first_object = first_row.storage_object_id.expect("content-linked object");
    assert_eq!(first_row.storage_rel_path, expected_key);

    // Identical bytes through a different external_id reuse the same object.
    let second = service
        .upsert_text_file_in_project(&group, &text_request("tw-B", "Beta", content))
        .await
        .expect("duplicate-content text upsert");
    let second_row = file_storage_row(&db, second.file_id).await;
    assert_eq!(second_row.storage_object_id, Some(first_object));
    assert_eq!(second_row.storage_rel_path, expected_key);

    // Physical bytes live exactly once at the canonical content key.
    let physical = storage_root.join(&expected_key);
    assert_eq!(std::fs::read(&physical).expect("stored bytes"), &stored[..]);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn text_replacement_releases_previous_content_only_after_reference_update() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping text replacement test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;

    let original = "original runbook\n";
    let replaced = "replacement runbook\n";
    let file = service
        .upsert_text_file_in_project(&group, &text_request("tw-R", "Runbook", original))
        .await
        .expect("initial text upsert");
    let before = file_storage_row(&db, file.file_id).await;
    let old_object = before.storage_object_id.expect("old object");
    let old_key = before.storage_rel_path.clone();
    assert!(storage_root.join(&old_key).exists());

    let updated = service
        .upsert_text_file_in_project(&group, &text_request("tw-R", "Runbook", replaced))
        .await
        .expect("text replacement");
    let after = file_storage_row(&db, updated.file_id).await;
    assert_ne!(after.storage_object_id, Some(old_object));
    // Expectations use trimmed content, matching the service's storage bytes.
    let replaced_stored = Bytes::from(replaced.trim());
    assert_eq!(
        after.storage_rel_path,
        format!("objects/{}/{}", group.id, sha256_hex(&replaced_stored))
    );

    // Old content is only released once nothing references it anymore.
    assert_eq!(storage_object_count(&db, old_object).await, 0);
    assert!(!storage_root.join(&old_key).exists());
    assert!(storage_root.join(&after.storage_rel_path).exists());
    assert_eq!(
        std::fs::read(storage_root.join(&after.storage_rel_path)).expect("stored bytes"),
        &replaced_stored[..]
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn legacy_direct_path_rows_stay_readable_and_deletable() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping legacy direct-path test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;

    let file_id = Uuid::new_v4();
    let rel_path = format!("{}/notes.txt", Uuid::new_v4());
    let content = b"legacy direct-path body\n".to_vec();
    let legacy_path = storage_root.join(&rel_path);
    std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
    std::fs::write(&legacy_path, &content).unwrap();

    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, \
          sha256, storage_rel_path, storage_object_id, ingest_status) \
         VALUES ($1, $2, 'public', NULL, $3, $4, 'text/plain', $5, $6, $7, NULL, 'succeeded')",
    )
    .bind(file_id)
    .bind(group.id)
    .bind("legacy-tw")
    .bind("notes.txt")
    .bind(content.len() as i64)
    .bind(sha256_hex(&content))
    .bind(&rel_path)
    .execute(db.pool())
    .await
    .expect("seed legacy direct-path row");

    // Reads still resolve through the direct path while storage_object_id is NULL.
    let detail = service
        .get_file_in_project(&group, file_id)
        .await
        .expect("legacy file detail");
    assert!(
        detail.source_available,
        "legacy direct path must stay readable"
    );

    // Deletes remove the direct-path object without touching storage objects.
    service
        .delete_file_in_project(&group, file_id)
        .await
        .expect("legacy file delete");
    assert!(!legacy_path.exists());
    let remaining_files: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.library_files WHERE group_id = $1")
            .bind(group.id)
            .fetch_one(db.pool())
            .await
            .expect("count remaining files");
    assert_eq!(remaining_files, 0);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
