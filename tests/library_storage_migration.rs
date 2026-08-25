//! Focused coverage for the legacy UUID direct-path storage migration tool:
//!
//! - dry runs must not touch database or storage state,
//! - successful runs link `storage_object_id`, rewrite `storage_rel_path` to
//!   the content-addressed key, keep the old key physically intact, and
//!   record it durably for the later cleanup phase,
//! - missing sources and size/hash mismatches never mutate anything,
//! - batches are bounded and reruns are idempotent,
//! - the reference update is conditional and cannot clobber concurrent writes,
//! - old-key cleanup records survive deletion of the source file row.
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch
//! database; they are skipped otherwise. Storage uses the local filesystem
//! backend only.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::Database;
use context69::library_store::LibraryStore;
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
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

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

/// The legacy migration selects rows across the whole database, so tests must
/// run one at a time inside this process and start from a selection set that
/// contains only their own fixtures. Stray direct-path rows left by earlier
/// or concurrent test suites (all test groups live under `full_path
/// LIKE 'test/%'`) are removed before each scenario.
static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn prepare_isolated_db() -> Option<(tokio::sync::MutexGuard<'static, ()>, Database)> {
    let guard = SUITE_LOCK.lock().await;
    let db = connect_db().await?;
    sqlx::query(
        "DELETE FROM context69.library_files \
         WHERE storage_object_id IS NULL \
           AND EXISTS (SELECT 1 FROM context69.groups AS g \
                       WHERE g.id = library_files.group_id AND g.full_path LIKE 'test/%')",
    )
    .execute(db.pool())
    .await
    .expect("purge stray legacy rows");
    Some((guard, db))
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
    .bind(format!("legacy-mig-{}", Uuid::new_v4()))
    .bind("Legacy Migration Test Group")
    .bind(format!("test/legacy-mig-{}", Uuid::new_v4()))
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
        "DELETE FROM context69.library_legacy_object_cleanup WHERE group_id = $1",
        "DELETE FROM context69.groups WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean up test rows");
    }
}

struct SeedLegacyOptions {
    size_bytes: Option<i64>,
    sha256: Option<String>,
    write_physical: bool,
}

/// Insert a legacy direct-path row and optionally its physical object under
/// the local storage root. Returns (file_id, old_rel_path).
async fn seed_legacy_file(
    db: &Database,
    storage_root: &std::path::Path,
    group_id: i64,
    external_id: &str,
    content: &[u8],
    options: &SeedLegacyOptions,
) -> (Uuid, String) {
    let file_id = Uuid::new_v4();
    let rel_path = format!("{}/notes.txt", Uuid::new_v4());
    let filename = format!("{external_id}.txt");
    if options.write_physical {
        let path = storage_root.join(&rel_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
    }
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, \
          sha256, storage_rel_path, storage_object_id, ingest_status) \
         VALUES ($1, $2, 'public', NULL, $3, $4, 'text/plain', $5, $6, $7, NULL, 'succeeded')",
    )
    .bind(file_id)
    .bind(group_id)
    .bind(external_id)
    .bind(&filename)
    .bind(options.size_bytes.unwrap_or(content.len() as i64))
    .bind(
        options
            .sha256
            .clone()
            .unwrap_or_else(|| sha256_hex(content)),
    )
    .bind(&rel_path)
    .execute(db.pool())
    .await
    .expect("seed legacy direct-path row");
    (file_id, rel_path)
}

async fn file_storage_row(db: &Database, file_id: Uuid) -> (String, Option<Uuid>) {
    let row = sqlx::query(
        "SELECT storage_rel_path, storage_object_id FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load file storage columns");
    (
        row.get("storage_rel_path"),
        row.get::<Option<Uuid>, _>("storage_object_id"),
    )
}

async fn cleanup_record_count(db: &Database, file_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM context69.library_legacy_object_cleanup WHERE file_id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("count cleanup records")
}

#[tokio::test]
async fn dry_run_reports_without_mutating_database_or_storage() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping dry-run test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"dry run body\n".to_vec();
    let (file_id, old_key) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-dry",
        &content,
        &SeedLegacyOptions {
            size_bytes: None,
            sha256: None,
            write_physical: true,
        },
    )
    .await;

    let summary = service
        .migrate_legacy_direct_paths(true, 10)
        .await
        .expect("dry-run migration");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.migrated, 1);
    assert_eq!(summary.errors, 0);

    // Nothing changed in the database or on disk.
    let (rel_path, object_id) = file_storage_row(&db, file_id).await;
    assert_eq!(rel_path, old_key);
    assert!(object_id.is_none());
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM context69.library_storage_objects WHERE group_id = $1",
        )
        .bind(group.id)
        .fetch_one(db.pool())
        .await
        .unwrap()
            == 0
    );
    assert_eq!(cleanup_record_count(&db, file_id).await, 0);
    assert!(storage_root.join(&old_key).exists());
    let sha = sha256_hex(&content);
    assert!(
        !storage_root
            .join(format!("objects/{}/{}", group.id, sha))
            .exists()
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn migration_links_content_addressed_object_and_records_old_key() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping linkage test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"migrate me\n".to_vec();
    let (file_id, old_key) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-link",
        &content,
        &SeedLegacyOptions {
            size_bytes: None,
            sha256: None,
            write_physical: true,
        },
    )
    .await;
    let sha = sha256_hex(&content);
    let expected_key = format!("objects/{}/{}", group.id, sha);

    let summary = service
        .migrate_legacy_direct_paths(false, 10)
        .await
        .expect("migration");
    assert_eq!(
        (summary.scanned, summary.migrated, summary.errors),
        (1, 1, 0)
    );

    // Reference update: object linked and rel_path rewritten to the object key.
    let (rel_path, object_id) = file_storage_row(&db, file_id).await;
    assert_eq!(rel_path, expected_key);
    let object_id = object_id.expect("content-addressed object linked");
    let row = sqlx::query(
        "SELECT group_id, sha256, size_bytes, storage_backend, object_key \
         FROM context69.library_storage_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("load storage object");
    let object_key: String = row.get("object_key");
    assert_eq!(row.get::<i64, _>("group_id"), group.id);
    assert_eq!(row.get::<String, _>("sha256"), sha);
    assert_eq!(row.get::<i64, _>("size_bytes"), content.len() as i64);
    assert_eq!(row.get::<String, _>("storage_backend"), "local");
    assert_eq!(object_key, expected_key);

    // Bytes verified at the content key; the old key is retained this phase.
    assert_eq!(
        std::fs::read(storage_root.join(&expected_key)).expect("stored bytes"),
        &content[..]
    );
    assert!(storage_root.join(&old_key).exists());

    // Durable old-key record committed together with the reference update.
    assert_eq!(cleanup_record_count(&db, file_id).await, 1);
    let record = sqlx::query(
        "SELECT group_id, old_key, deleted_at FROM context69.library_legacy_object_cleanup \
         WHERE file_id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load cleanup record");
    assert_eq!(record.get::<i64, _>("group_id"), group.id);
    assert_eq!(record.get::<String, _>("old_key"), old_key);
    assert!(
        record
            .get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")
            .is_none()
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn missing_source_is_reported_without_any_mutation() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping missing-source test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (file_id, old_key) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-missing",
        b"never stored\n",
        &SeedLegacyOptions {
            size_bytes: None,
            sha256: None,
            write_physical: false,
        },
    )
    .await;

    let summary = service
        .migrate_legacy_direct_paths(false, 10)
        .await
        .expect("migration");
    assert_eq!(
        (
            summary.scanned,
            summary.missing,
            summary.migrated,
            summary.invalid,
            summary.errors,
            summary.conflicts
        ),
        (1, 1, 0, 0, 0, 0)
    );

    let (rel_path, object_id) = file_storage_row(&db, file_id).await;
    assert_eq!(rel_path, old_key);
    assert!(object_id.is_none());
    assert_eq!(cleanup_record_count(&db, file_id).await, 0);

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn bad_size_or_hash_sources_are_rejected_and_retained() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping validation test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let wrong_size = b"wrong recorded size\n".to_vec();
    let (wrong_size_id, wrong_size_key) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-size",
        &wrong_size,
        &SeedLegacyOptions {
            size_bytes: Some(wrong_size.len() as i64 + 5),
            sha256: None,
            write_physical: true,
        },
    )
    .await;
    let wrong_hash = b"wrong recorded hash\n".to_vec();
    let (wrong_hash_id, wrong_hash_key) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-hash",
        &wrong_hash,
        &SeedLegacyOptions {
            size_bytes: None,
            sha256: Some("a".repeat(64)),
            write_physical: true,
        },
    )
    .await;

    let summary = service
        .migrate_legacy_direct_paths(false, 10)
        .await
        .expect("migration");
    assert_eq!(
        (summary.scanned, summary.invalid, summary.migrated),
        (2, 2, 0)
    );

    for (file_id, old_key) in [
        (wrong_size_id, wrong_size_key),
        (wrong_hash_id, wrong_hash_key),
    ] {
        let (rel_path, object_id) = file_storage_row(&db, file_id).await;
        assert_eq!(rel_path, old_key);
        assert!(object_id.is_none());
        assert!(
            storage_root.join(&old_key).exists(),
            "old object must be retained"
        );
        assert_eq!(cleanup_record_count(&db, file_id).await, 0);
    }

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn bounded_batches_are_restart_safe_and_idempotent() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping restart test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let mut seeded = Vec::new();
    for index in 0..3 {
        let content = format!("restart body {index}\n").into_bytes();
        seeded.push(
            seed_legacy_file(
                &db,
                &storage_root,
                group.id,
                &format!("legacy-restart-{index}"),
                &content,
                &SeedLegacyOptions {
                    size_bytes: None,
                    sha256: None,
                    write_physical: true,
                },
            )
            .await,
        );
    }

    // Batch size 1 forces multiple internal selection pages; a permanently
    // failing row could not block later pages thanks to the cursor ordering.
    let first_pass = service
        .migrate_legacy_direct_paths(false, 1)
        .await
        .expect("first migration pass");
    assert_eq!(
        (first_pass.scanned, first_pass.migrated, first_pass.errors),
        (3, 3, 0)
    );

    // A restart rescans from the beginning but finds nothing left to do.
    let second_pass = service
        .migrate_legacy_direct_paths(false, 1)
        .await
        .expect("second migration pass");
    assert_eq!(
        (
            second_pass.scanned,
            second_pass.migrated,
            second_pass.already_migrated
        ),
        (0, 0, 0)
    );

    // Cleanup records exist exactly once per migrated row.
    for (file_id, _) in &seeded {
        assert_eq!(cleanup_record_count(&db, *file_id).await, 1);
    }

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn legacy_reference_update_is_conditional() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping conditional-update test");
        return;
    };
    let store = LibraryStore::new(db.clone());
    let group = seed_group_record(&db).await;
    let sha = "c".repeat(64);
    let object_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, 12, 'local', $4)",
    )
    .bind(object_id)
    .bind(group.id)
    .bind(&sha)
    .bind(format!("objects/{}/{}", group.id, sha))
    .execute(db.pool())
    .await
    .expect("insert storage object");

    // Legacy row: still direct-path, so the guarded update lands.
    let legacy_id = Uuid::new_v4();
    let old_key = format!("{}/guarded.txt", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, visibility, filename, media_type, size_bytes, sha256, \
          storage_rel_path, storage_object_id, ingest_status) \
         VALUES ($1, $2, 'public', 'guarded.txt', 'text/plain', 12, $3, $4, NULL, 'succeeded')",
    )
    .bind(legacy_id)
    .bind(group.id)
    .bind(&sha)
    .bind(&old_key)
    .execute(db.pool())
    .await
    .expect("insert legacy row");

    // A stale expected key must not match.
    let mut tx = db.pool().begin().await.expect("begin tx");
    let landed_wrong_key = store
        .link_legacy_file_storage_object_on_connection(
            &mut tx,
            legacy_id,
            "objects/someone-else/old",
            object_id,
            &format!("objects/{}/{}", group.id, sha),
        )
        .await
        .expect("conditional update with stale key");
    assert!(!landed_wrong_key, "stale old key must be rejected");

    // The exact old key links the row inside the same transaction shape.
    let landed = store
        .link_legacy_file_storage_object_on_connection(
            &mut tx,
            legacy_id,
            &old_key,
            object_id,
            &format!("objects/{}/{}", group.id, sha),
        )
        .await
        .expect("conditional update");
    assert!(landed, "matching old key must link the row");
    tx.commit().await.expect("commit tx");

    // Once linked, further attempts (e.g. a racing worker) find nothing.
    let mut tx = db.pool().begin().await.expect("begin tx");
    let raced = store
        .link_legacy_file_storage_object_on_connection(
            &mut tx,
            legacy_id,
            &old_key,
            object_id,
            &format!("objects/{}/{}", group.id, sha),
        )
        .await
        .expect("racing conditional update");
    tx.rollback().await.ok();
    assert!(!raced, "already-linked row must reject another update");

    cleanup_group(&db, group.id).await;
}

#[tokio::test]
async fn reference_count_distinguishes_shared_from_unreferenced_objects() {
    // Covers the verification behind the migration tool's guarded cleanup:
    // an object whose link failed must be removable, while a shared object
    // (including one linked by a transaction that actually committed despite
    // reporting an error) must be retained.
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping reference-count test");
        return;
    };
    let store = LibraryStore::new(db.clone());
    let group = seed_group_record(&db).await;
    let sha = "d".repeat(64);
    let object_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key) \
         VALUES ($1, $2, $3, 4, 'local', $4)",
    )
    .bind(object_id)
    .bind(group.id)
    .bind(&sha)
    .bind(format!("objects/{}/{}", group.id, sha))
    .execute(db.pool())
    .await
    .expect("insert storage object");

    assert_eq!(
        store
            .count_storage_object_references(object_id)
            .await
            .expect("count references"),
        0,
        "unlinked object must report zero references"
    );

    for external_id in ["refcount-a", "refcount-b"] {
        store
            .create_file_in_project(
                group.id,
                &context69::library_store::NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some(external_id.to_string()),
                    filename: format!("{external_id}.txt"),
                    media_type: "text/plain".to_string(),
                    size_bytes: 4,
                    sha256: sha.clone(),
                    storage_rel_path: format!("objects/{}/{}", group.id, sha),
                    storage_object_id: Some(object_id),
                },
            )
            .await
            .expect("create referencing file");
    }
    assert_eq!(
        store
            .count_storage_object_references(object_id)
            .await
            .expect("count references after linking"),
        2,
        "shared object must report every library_files reference"
    );

    cleanup_group(&db, group.id).await;
}

#[tokio::test]
async fn old_key_cleanup_record_survives_file_deletion() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping durability test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"outlive me\n".to_vec();
    let (file_id, _) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-durable",
        &content,
        &SeedLegacyOptions {
            size_bytes: None,
            sha256: None,
            write_physical: true,
        },
    )
    .await;

    let summary = service
        .migrate_legacy_direct_paths(false, 10)
        .await
        .expect("migration");
    assert_eq!(summary.migrated, 1);

    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("delete source file row");

    assert_eq!(
        cleanup_record_count(&db, file_id).await,
        1,
        "cleanup record must survive file deletion"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn startup_helper_migrates_with_default_batch_and_is_restart_safe() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping startup-helper test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let content = b"startup migrated\n".to_vec();
    let (file_id, old_key) = seed_legacy_file(
        &db,
        &storage_root,
        group.id,
        "legacy-startup",
        &content,
        &SeedLegacyOptions {
            size_bytes: None,
            sha256: None,
            write_physical: true,
        },
    )
    .await;
    let sha = sha256_hex(&content);
    let expected_key = format!("objects/{}/{}", group.id, sha);

    // The startup entry point uses the safe default batch size and performs real
    // writes (never a dry run), linking the content-addressed object.
    let summary = service
        .run_startup_legacy_migration()
        .await
        .expect("startup migration");
    assert_eq!(
        (summary.scanned, summary.migrated, summary.errors),
        (1, 1, 0)
    );
    let (rel_path, object_id) = file_storage_row(&db, file_id).await;
    assert_eq!(rel_path, expected_key);
    assert!(object_id.is_some());
    assert!(storage_root.join(&old_key).exists(), "old key retained");
    assert_eq!(cleanup_record_count(&db, file_id).await, 1);

    // A restart finds nothing left to do; the existing object is reused.
    let second = service
        .run_startup_legacy_migration()
        .await
        .expect("startup migration restart");
    assert_eq!(
        (second.scanned, second.migrated, second.already_migrated),
        (0, 0, 0)
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
