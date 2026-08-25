//! Focused coverage for the legacy old-key cleanup tool:
//!
//! - dry runs report eligibility without any database or storage mutation,
//! - execute deletes the physical old object and closes the record,
//! - an old key still referenced by `library_files.storage_rel_path` is
//!   never deleted and its record stays open,
//! - records written for another storage backend, or with an unknown
//!   (pre-0024) backend, are skipped and never deleted,
//! - an already-missing object is an idempotent success that closes the
//!   record (the retry path after a failed DB mark),
//! - batches are bounded and reruns are idempotent,
//! - error recording and conditional success marking behave at the store
//!   level.
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch
//! database with the current migrations applied; they are skipped otherwise.
//! Storage uses the local filesystem backend only.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
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

/// Cleanup selection spans the whole database, so tests run one at a time and
/// start from a selection set containing only their own fixtures.
static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn prepare_isolated_db() -> Option<(tokio::sync::MutexGuard<'static, ()>, Database)> {
    let guard = SUITE_LOCK.lock().await;
    let db = connect_db().await?;
    sqlx::query(
        "DELETE FROM context69.library_legacy_object_cleanup AS c \
         USING context69.groups AS g \
         WHERE c.group_id = g.id AND g.full_path LIKE 'test/%'",
    )
    .execute(db.pool())
    .await
    .expect("purge stray cleanup records");
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

async fn seed_group_record(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'public', 'shared', $3) RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(format!("legacy-cleanup-{}", Uuid::new_v4()))
    .bind("Legacy Cleanup Test Group")
    .bind(format!("test/legacy-cleanup-{}", Uuid::new_v4()))
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

struct SeedCleanupOptions {
    /// Storage backend recorded on the cleanup record; `None` leaves the
    /// column NULL (a pre-0024 row with unknown backend).
    old_storage_backend: Option<&'static str>,
    /// Whether the grace period has elapsed; defaults to true (eligible).
    eligible: bool,
    /// Whether a prior run persisted a delete error on this record.
    with_delete_error: bool,
}

/// Insert one open cleanup record plus its physical old object under the
/// local storage root. Returns (record_id, file_id, old_key).
async fn seed_cleanup_record(
    db: &Database,
    storage_root: &std::path::Path,
    group_id: i64,
    label: &str,
    content: &[u8],
    options: &SeedCleanupOptions,
) -> (i64, Uuid, String) {
    let file_id = Uuid::new_v4();
    let old_key = format!("{}/{label}.txt", Uuid::new_v4());
    if let Some(parent) = std::path::Path::new(&old_key).parent() {
        let physical = storage_root.join(parent);
        std::fs::create_dir_all(physical).unwrap();
    }
    std::fs::write(storage_root.join(&old_key), content).unwrap();
    let backend = options.old_storage_backend;
    let eligible_at = if options.eligible {
        Utc::now() - ChronoDuration::hours(1)
    } else {
        Utc::now() + ChronoDuration::days(7)
    };
    let row = sqlx::query(
        "INSERT INTO context69.library_legacy_object_cleanup \
         (group_id, file_id, old_key, old_storage_backend, cleanup_eligible_at, delete_error) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(group_id)
    .bind(file_id)
    .bind(&old_key)
    .bind(backend)
    .bind(eligible_at)
    .bind(
        options
            .with_delete_error
            .then(|| "prior attempt failed".to_string()),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed cleanup record");
    (row.get::<i64, _>("id"), file_id, old_key)
}

/// Insert a live library_files row still pointing at `rel_path` so the old
/// key counts as referenced.
async fn seed_referencing_file(db: &Database, group_id: i64, rel_path: &str) -> Uuid {
    let file_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, \
          sha256, storage_rel_path, storage_object_id, ingest_status) \
         VALUES ($1, $2, 'public', NULL, NULL, 'holder.txt', 'text/plain', 0, $3, $4, NULL, 'succeeded')",
    )
    .bind(file_id)
    .bind(group_id)
    .bind(format!("{:064x}", 0))
    .bind(rel_path)
    .execute(db.pool())
    .await
    .expect("seed referencing file");
    file_id
}

async fn record_state(
    db: &Database,
    record_id: i64,
) -> (Option<chrono::DateTime<chrono::Utc>>, Option<String>) {
    let row = sqlx::query(
        "SELECT deleted_at, delete_error FROM context69.library_legacy_object_cleanup \
         WHERE id = $1",
    )
    .bind(record_id)
    .fetch_one(db.pool())
    .await
    .expect("load cleanup record");
    (row.get("deleted_at"), row.get("delete_error"))
}

#[tokio::test]
async fn dry_run_reports_eligible_without_mutating_anything() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping dry-run test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "dry-run",
        b"keep me\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;

    let summary = service
        .cleanup_legacy_objects(false, 10)
        .await
        .expect("dry run");
    assert_eq!(
        (
            summary.scanned,
            summary.eligible,
            summary.deleted,
            summary.already_missing,
            summary.skipped_referenced,
            summary.skipped_backend,
            summary.errors
        ),
        (1, 1, 0, 0, 0, 0, 0)
    );

    // Nothing changed: object intact, record still open without errors.
    assert!(
        storage_root.join(&old_key).exists(),
        "object must survive dry run"
    );
    let (deleted_at, delete_error) = record_state(&db, record_id).await;
    assert!(deleted_at.is_none());
    assert!(delete_error.is_none());

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn execute_deletes_object_and_closes_record() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping execute test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "execute",
        b"delete me\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;

    let summary = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("cleanup");
    assert_eq!(
        (
            summary.scanned,
            summary.eligible,
            summary.deleted,
            summary.errors
        ),
        (1, 1, 1, 0)
    );
    assert!(!storage_root.join(&old_key).exists(), "object must be gone");
    let (deleted_at, delete_error) = record_state(&db, record_id).await;
    assert!(deleted_at.is_some(), "record must be marked deleted");
    assert!(delete_error.is_none());

    // Rerun is a no-op: closed rows drop out of the selection set.
    let rerun = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("rerun");
    assert_eq!(
        (
            rerun.scanned,
            rerun.deleted,
            rerun.already_missing,
            rerun.errors
        ),
        (0, 0, 0, 0)
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn referenced_old_key_is_skipped_and_stays_open() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping referenced-skip test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "referenced",
        b"still used\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;
    seed_referencing_file(&db, group.id, &old_key).await;

    // Dry run must also see the reference and not count it as eligible.
    let dry = service
        .cleanup_legacy_objects(false, 10)
        .await
        .expect("dry run");
    assert_eq!(
        (dry.scanned, dry.eligible, dry.skipped_referenced),
        (1, 0, 1)
    );

    let summary = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("cleanup");
    assert_eq!(
        (
            summary.scanned,
            summary.eligible,
            summary.deleted,
            summary.skipped_referenced,
            summary.errors
        ),
        (1, 0, 0, 1, 0)
    );
    assert!(
        storage_root.join(&old_key).exists(),
        "referenced object must never be deleted"
    );
    let (deleted_at, _) = record_state(&db, record_id).await;
    assert!(
        deleted_at.is_none(),
        "skipped rows must not be marked deleted"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn backend_mismatch_is_skipped() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping backend-mismatch test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "other-backend",
        b"s3 only\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("s3"),
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;

    let summary = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("cleanup");
    assert_eq!(
        (
            summary.scanned,
            summary.eligible,
            summary.deleted,
            summary.skipped_backend,
            summary.errors
        ),
        (1, 0, 0, 1, 0)
    );
    assert!(
        storage_root.join(&old_key).exists(),
        "mismatched-backend object must be untouched"
    );
    let (deleted_at, _) = record_state(&db, record_id).await;
    assert!(deleted_at.is_none());

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn unknown_backend_is_skipped_and_not_deleted() {
    // Rows recorded before migration 0024 have no stored backend; the
    // cleanup phase must never guess and delete from the active store.
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping unknown-backend test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "unknown-backend",
        b"unknown origin\n",
        &SeedCleanupOptions {
            old_storage_backend: None,
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;

    // Dry run must also refuse to count the record as eligible.
    let dry = service
        .cleanup_legacy_objects(false, 10)
        .await
        .expect("dry run");
    assert_eq!((dry.scanned, dry.eligible, dry.skipped_backend), (1, 0, 1));

    let summary = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("cleanup");
    assert_eq!(
        (
            summary.scanned,
            summary.eligible,
            summary.deleted,
            summary.skipped_backend,
            summary.errors
        ),
        (1, 0, 0, 1, 0)
    );
    assert!(
        storage_root.join(&old_key).exists(),
        "unknown-backend object must be untouched"
    );
    let (deleted_at, _) = record_state(&db, record_id).await;
    assert!(
        deleted_at.is_none(),
        "skipped rows must not be marked deleted"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn missing_object_is_an_idempotent_success() {
    // Covers both the pre-deleted-object case and the retry path after a
    // failed DB mark: the physical delete succeeded but the record stayed
    // open, so the next run observes absence, reports it, and closes the row.
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping missing-object test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "vanished",
        b"already gone\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;
    std::fs::remove_file(storage_root.join(&old_key)).unwrap();

    let summary = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("cleanup");
    assert_eq!(
        (
            summary.scanned,
            summary.eligible,
            summary.deleted,
            summary.already_missing,
            summary.errors
        ),
        (1, 1, 0, 1, 0)
    );
    let (deleted_at, _) = record_state(&db, record_id).await;
    assert!(
        deleted_at.is_some(),
        "idempotent miss must close the record"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn bounded_batches_are_restart_safe_and_idempotent() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping batch test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let mut seeded = Vec::new();
    for index in 0..3 {
        seeded.push(
            seed_cleanup_record(
                &db,
                &storage_root,
                group.id,
                &format!("batch-{index}"),
                format!("batch body {index}\n").as_bytes(),
                &SeedCleanupOptions {
                    old_storage_backend: Some("local"),
                    eligible: true,
                    with_delete_error: false,
                },
            )
            .await,
        );
    }
    // A not-yet-eligible record must stay open across every pass.
    let (_, future_file_id, _) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "future",
        b"not yet\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: false,
            with_delete_error: false,
        },
    )
    .await;

    // Batch size 1 forces multiple internal selection pages; the id cursor
    // keeps errored rows from blocking later pages.
    let first_pass = service
        .cleanup_legacy_objects(true, 1)
        .await
        .expect("first pass");
    assert_eq!(
        (first_pass.scanned, first_pass.deleted, first_pass.errors),
        (3, 3, 0)
    );
    let second_pass = service
        .cleanup_legacy_objects(true, 1)
        .await
        .expect("second pass");
    assert_eq!(
        (
            second_pass.scanned,
            second_pass.deleted,
            second_pass.already_missing,
            second_pass.errors
        ),
        (0, 0, 0, 0)
    );

    for (record_id, _, _) in &seeded {
        let (deleted_at, _) = record_state(&db, *record_id).await;
        assert!(deleted_at.is_some());
    }
    let future_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.library_legacy_object_cleanup \
         WHERE file_id = $1 AND deleted_at IS NULL",
    )
    .bind(future_file_id)
    .fetch_one(db.pool())
    .await
    .expect("count future records");
    assert_eq!(future_rows, 1, "grace-period record must remain open");

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn error_recording_and_conditional_mark_behave_at_store_level() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping store-helper test");
        return;
    };
    let store = LibraryStore::new(db.clone());
    let group = seed_group_record(&db).await;
    let scratch_root = std::env::temp_dir().join(format!("context69-test-{}", Uuid::new_v4()));
    let (record_id, file_id, old_key) = seed_cleanup_record(
        &db,
        &scratch_root,
        group.id,
        "store-helpers",
        b"helpers\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: true,
            with_delete_error: false,
        },
    )
    .await;
    assert!(
        scratch_root.join(&old_key).exists(),
        "seed must place a physical object"
    );

    // A recorded failure keeps the row open and selected for retry.
    store
        .record_legacy_cleanup_error(record_id, "transient storage outage")
        .await
        .expect("record delete error");
    let page = store
        .list_eligible_legacy_cleanup(None, 100)
        .await
        .expect("list eligible");
    assert!(page.iter().any(|row| row.id == record_id));
    assert!(page.iter().any(|row| row.old_key == old_key));

    // Conditional success mark lands once and only once.
    assert!(
        store
            .mark_legacy_cleanup_deleted(record_id)
            .await
            .expect("mark deleted"),
        "first conditional mark must land"
    );
    assert!(
        !store
            .mark_legacy_cleanup_deleted(record_id)
            .await
            .expect("second mark"),
        "closed record must reject another mark"
    );
    let (deleted_at, delete_error) = record_state(&db, record_id).await;
    assert!(deleted_at.is_some());
    assert_eq!(delete_error.as_deref(), None, "success clears stale errors");

    // Closed rows leave the retry selection set.
    let page = store
        .list_eligible_legacy_cleanup(None, 100)
        .await
        .expect("list eligible after close");
    assert!(!page.iter().any(|row| row.file_id == file_id));
    assert!(
        scratch_root.join(&old_key).exists(),
        "store-level helpers never touch physical storage"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(scratch_root);
}

#[tokio::test]
async fn retry_after_prior_failure_clears_error_and_succeeds() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping retry test");
        return;
    };
    let (service, storage_root) = build_library_service(&db).await;
    let group = seed_group_record(&db).await;
    let (record_id, _, old_key) = seed_cleanup_record(
        &db,
        &storage_root,
        group.id,
        "retry",
        b"retry me\n",
        &SeedCleanupOptions {
            old_storage_backend: Some("local"),
            eligible: true,
            with_delete_error: true,
        },
    )
    .await;

    let summary = service
        .cleanup_legacy_objects(true, 10)
        .await
        .expect("cleanup retry");
    assert_eq!(
        (summary.deleted, summary.errors),
        (1, 0),
        "a previously failed row must be retried"
    );
    assert!(!storage_root.join(&old_key).exists());
    let (deleted_at, delete_error) = record_state(&db, record_id).await;
    assert!(deleted_at.is_some());
    assert!(
        delete_error.is_none(),
        "successful retry clears delete_error"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
