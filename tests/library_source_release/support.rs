//! Fixtures for the issue 332 phase 2 source-release integration tests.
//!
//! Seeding, service construction, and cleanup only; assertions stay in the
//! case files. The local filesystem storage backend is used so a test can
//! observe whether the physical source object still exists.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
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
use sqlx::Row;
use uuid::Uuid;

pub use crate::support_seed::*;

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

pub async fn connect_scratch() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

pub async fn build_library_service(db: &Database) -> (LibraryService, std::path::PathBuf) {
    let storage_root = std::env::temp_dir().join(format!("context69-release-{}", Uuid::new_v4()));
    let service = build_library_service_at(db, storage_root.clone()).await;
    (service, storage_root)
}

/// Build a service over an explicit storage root; used to model a process
/// restart that must recover durable intents from the database.
pub async fn build_library_service_at(
    db: &Database,
    storage_root: std::path::PathBuf,
) -> LibraryService {
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
    LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            file_library: FileLibraryConfig {
                storage_root,
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
    .expect("build library service")
}

pub async fn seed_group_record(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) \
         RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(format!("source-release-{}", Uuid::new_v4()))
    .bind("Source Release Test Group")
    .bind(format!("test/source-release-{}", Uuid::new_v4()))
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

/// Insert a content-addressed storage object row and write its bytes to the
/// local backend. Returns the object id and its object key.
pub async fn seed_storage_object(
    db: &Database,
    storage_root: &std::path::Path,
    group_id: i64,
    sha256: &str,
    content: &[u8],
) -> (Uuid, String) {
    let object_id = Uuid::new_v4();
    let object_key = format!("objects/{group_id}/{sha256}");
    let row = sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, $4, 'local', $5) RETURNING object_key",
    )
    .bind(object_id)
    .bind(group_id)
    .bind(sha256)
    .bind(content.len() as i64)
    .bind(&object_key)
    .fetch_one(db.pool())
    .await
    .expect("seed storage object");
    let stored_key: String = row.get("object_key");
    write_object(storage_root, &stored_key, content);
    (object_id, stored_key)
}

pub fn write_object(storage_root: &std::path::Path, key: &str, content: &[u8]) {
    let path = storage_root.join(key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create object parent");
    }
    std::fs::write(path, content).expect("write object bytes");
}

pub struct SeedFileOptions {
    pub sha256: String,
    pub content: Vec<u8>,
    pub ingest_status: String,
    pub delete_source_after_processing: bool,
    pub source_released: bool,
    pub storage_object_id: Option<Uuid>,
    pub storage_rel_path: OrphanKey,
}

/// How a seeded row points at storage.
pub enum OrphanKey {
    /// Content-addressed key produced by [`seed_storage_object`].
    Object(String),
    /// A legacy direct-path key with no storage-object row.
    Legacy(String),
}

pub async fn seed_file(db: &Database, group_id: i64, options: SeedFileOptions) -> Uuid {
    let file_id = Uuid::new_v4();
    let storage_rel_path = match &options.storage_rel_path {
        OrphanKey::Object(key) | OrphanKey::Legacy(key) => key.clone(),
    };
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, filename, media_type, size_bytes, sha256, storage_rel_path, \
          storage_object_id, ingest_status, delete_source_after_processing, source_released_at, \
          visibility) \
         VALUES ($1, $2, $3, 'text/plain', $4, $5, $6, $7, $8, $9, \
                 CASE WHEN $10 THEN now() ELSE NULL END, 'public')",
    )
    .bind(file_id)
    .bind(group_id)
    .bind(format!("release-{file_id}.txt"))
    .bind(options.content.len() as i64)
    .bind(&options.sha256)
    .bind(&storage_rel_path)
    .bind(options.storage_object_id)
    .bind(&options.ingest_status)
    .bind(options.delete_source_after_processing)
    .bind(options.source_released)
    .execute(db.pool())
    .await
    .expect("seed library file");
    file_id
}

/// Seed a legacy direct-path row with no storage object (eligible for the
/// missing-source cleanup checks).
pub async fn seed_legacy_file(
    db: &Database,
    group_id: i64,
    storage_rel_path: &str,
    released: bool,
) -> Uuid {
    seed_file(
        db,
        group_id,
        SeedFileOptions {
            sha256: "a".repeat(64),
            content: b"legacy".to_vec(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: released,
            storage_object_id: None,
            storage_rel_path: OrphanKey::Legacy(storage_rel_path.to_string()),
        },
    )
    .await
}

pub async fn release_state(
    db: &Database,
    file_id: Uuid,
) -> (bool, Option<chrono::DateTime<chrono::Utc>>) {
    let row = sqlx::query(
        "SELECT source_released_at, (storage_object_id IS NULL) AS detached \
         FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load release state");
    (
        row.get::<bool, _>("detached"),
        row.get("source_released_at"),
    )
}

pub async fn object_row_exists(db: &Database, object_id: Uuid) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM context69.library_storage_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("count object rows")
        > 0
}

pub async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query(
        "DELETE FROM context69.library_file_documents WHERE file_id IN \
         (SELECT id FROM context69.library_files WHERE group_id = $1)",
    )
    .bind(group_id)
    .execute(db.pool())
    .await
    .ok();
    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query(
        "DELETE FROM context69.task_items WHERE task_id IN \
         (SELECT id FROM context69.tasks WHERE group_id = $1)",
    )
    .bind(group_id)
    .execute(db.pool())
    .await
    .ok();
    sqlx::query("DELETE FROM context69.tasks WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up tasks");
    sqlx::query("DELETE FROM context69.library_files WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up files");
    sqlx::query("DELETE FROM context69.library_storage_objects WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up storage objects");
    sqlx::query("DELETE FROM context69.library_storage_object_cleanup WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up source object cleanup intents");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

pub async fn seed_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'unused') RETURNING id",
    )
    .bind(format!("source-release-{}", Uuid::new_v4()))
    .bind("Source Release Test")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

pub async fn cleanup_user(db: &Database, user_id: i64) {
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .ok();
}
