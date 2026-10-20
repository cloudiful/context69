//! Issue #975 P2: artifact references across every object lifecycle path.
//!
//! These tests run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are ignored by a plain
//! `cargo test --workspace` and reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.
//!
//! Coverage:
//! - the artifact reference counts and protects against both guarded delete
//!   statements, the staged orphan sweep and the staging-lease clear;
//! - file, task-input and artifact references share one object safely;
//! - artifact deletion, retarget and task/item cascade enqueue the exact old
//!   registry identity through the existing durable outbox, dedupe, and fire no
//!   intent for a same-object update, a staged object or a rolled-back mutation;
//! - the real cleanup worker keeps bytes while any reference remains and
//!   physically deletes them after the last reference disappears;
//! - a cleanup identity always comes from the object registry, never from the
//!   owning task's group, and a mismatched intent deletes nothing;
//! - a concurrent new artifact reference waits for the cleanup identity lock.
//!
//! The local filesystem storage backend is used throughout; no S3 alias or
//! bucket is touched.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, SubsecRound, Utc};
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::{Database, TaskItemArtifactKind};
use context69::library_store::LibraryStore;
use context69::services::library::{LibraryService, LibraryServiceConfig, SourceCleanupDispatcher};
use context69::services::settings::SettingsService;
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Object selection and open-intent purging span the whole database, so the
/// suite runs one test at a time.
static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const KIND_DOWNLOAD: &str = "download";
const KIND_SECTIONS: &str = "sections";
const ENCODING_DOWNLOAD: &str = "raw-v1";
const ENCODING_SECTIONS: &str = "ingest-sections-json-v1";

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

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();

    Database::connect(&url)
        .await
        .expect("connect test database")
}

async fn prepare() -> (tokio::sync::MutexGuard<'static, ()>, Database) {
    let guard = SUITE_LOCK.lock().await;
    let db = connect_scratch().await;
    sqlx::query_file!("tests/task_item_artifacts/purge_open_intents.sql")
        .execute(db.pool())
        .await
        .expect("purge stray open cleanup intents");
    (guard, db)
}

async fn build_service(db: &Database, storage_root: &Path) -> LibraryService {
    let settings = SettingsService::new(db.clone());
    let translation = TranslationService::new(TranslationDependencies {
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
                storage_root: storage_root.to_path_buf(),
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
    )
    .await
    .expect("build library service")
}

struct Owner {
    user_id: i64,
    group_id: i64,
}

async fn seed_owner(db: &Database) -> Owner {
    let user_id: i64 = sqlx::query_file!(
        "tests/task_item_artifacts/seed_user.sql",
        format!("task-artifact-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .id;
    let group_id: i64 = sqlx::query_file!(
        "tests/task_item_artifacts/seed_group.sql",
        format!("task-artifact-{}", Uuid::new_v4()),
        format!("Task Artifact {}", Uuid::new_v4()),
        format!("test/task-artifact-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .id;
    Owner { user_id, group_id }
}

async fn seed_task_item(db: &Database, owner: &Owner) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_task.sql",
        task_id,
        owner.user_id,
        owner.group_id
    )
    .execute(db.pool())
    .await
    .expect("seed task");
    let item_id = Uuid::new_v4();
    sqlx::query_file!("tests/task_item_artifacts/seed_item.sql", item_id, task_id)
        .execute(db.pool())
        .await
        .expect("seed task item");
    (task_id, item_id)
}

fn hex_digest() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Insert a content-addressed storage object and write its local bytes.
async fn seed_object(
    db: &Database,
    storage_root: &Path,
    group_id: i64,
    content: &[u8],
) -> (Uuid, String, String) {
    let object_id = Uuid::new_v4();
    let sha256 = hex_digest();
    let object_key = format!("objects/{group_id}/{sha256}");
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_storage_object.sql",
        object_id,
        group_id,
        sha256.clone(),
        content.len() as i64,
        object_key.clone()
    )
    .execute(db.pool())
    .await
    .expect("seed storage object");
    write_object(storage_root, &object_key, content);
    (object_id, object_key, sha256)
}

fn write_object(storage_root: &Path, key: &str, content: &[u8]) {
    let path = storage_root.join(key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create object parent");
    }
    std::fs::write(path, content).expect("write object bytes");
}

async fn seed_artifact(db: &Database, item_id: Uuid, kind: &str, object_id: Uuid) {
    let encoding = if kind == KIND_DOWNLOAD {
        ENCODING_DOWNLOAD
    } else {
        ENCODING_SECTIONS
    };
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_artifact.sql",
        item_id,
        kind,
        object_id,
        encoding
    )
    .execute(db.pool())
    .await
    .expect("seed artifact reference");
}

async fn seed_file_reference(db: &Database, group_id: i64, object_id: Uuid, key: &str, sha: &str) {
    let file_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_file_reference.sql",
        file_id,
        group_id,
        format!("artifact-{file_id}.txt"),
        4_i64,
        sha,
        key,
        object_id
    )
    .execute(db.pool())
    .await
    .expect("seed file reference");
}

async fn seed_task_input_reference(db: &Database, item_id: Uuid, object_id: Uuid) {
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_task_input_reference.sql",
        item_id,
        object_id
    )
    .execute(db.pool())
    .await
    .expect("seed task input reference");
}

async fn object_rows(db: &Database, object_id: Uuid) -> i64 {
    sqlx::query_file!("tests/task_item_artifacts/object_exists.sql", object_id)
        .fetch_one(db.pool())
        .await
        .expect("count object rows")
        .count
        .expect("object row count")
}

async fn staging_lease(db: &Database, object_id: Uuid) -> Option<DateTime<Utc>> {
    sqlx::query_file!(
        "tests/task_item_artifacts/object_staging_lease.sql",
        object_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read staging lease")
    .staging_lease_until
}

async fn set_staging_lease(db: &Database, object_id: Uuid, lease: Option<DateTime<Utc>>) {
    sqlx::query_file!(
        "tests/task_item_artifacts/set_staging_lease.sql",
        object_id,
        lease
    )
    .execute(db.pool())
    .await
    .expect("set staging lease");
}

/// Move a staged object past the production 24-hour sweep grace and to the
/// front of its bounded ordering page.
async fn age_staged_object(db: &Database, object_id: Uuid, lease: DateTime<Utc>) {
    sqlx::query_file!(
        "tests/task_item_artifacts/age_staged_object.sql",
        object_id,
        lease
    )
    .execute(db.pool())
    .await
    .expect("age the staged object");
}

async fn backend_pid(connection: &mut sqlx::PgConnection) -> i32 {
    sqlx::query_file!("tests/task_item_artifacts/backend_pid.sql")
        .fetch_one(connection)
        .await
        .expect("read the backend pid")
        .pid
}

async fn object_lock_waiter_count(db: &Database, blocker_pid: i32) -> i64 {
    sqlx::query_file!(
        "tests/task_item_artifacts/object_lock_waiters.sql",
        blocker_pid
    )
    .fetch_one(db.pool())
    .await
    .expect("count storage-object lock waiters")
    .count
}

/// Bounded visibility probe: proves the cleanup backend is actually waiting on
/// the transaction that still holds the storage-object row lock.
async fn wait_for_object_lock_waiter(db: &Database, blocker_pid: i32) -> bool {
    for _ in 0..500 {
        if object_lock_waiter_count(db, blocker_pid).await > 0 {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    false
}

async fn wait_until_object_is_gone(db: &Database, object_id: Uuid) -> bool {
    for _ in 0..1500 {
        if object_rows(db, object_id).await == 0 {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    false
}

async fn open_intents(db: &Database, object_id: Uuid) -> i64 {
    sqlx::query_file!("tests/task_item_artifacts/open_intents.sql", object_id)
        .fetch_one(db.pool())
        .await
        .expect("count open intents")
        .count
        .expect("open intent count")
}

async fn completed_intents(db: &Database, object_id: Uuid) -> i64 {
    sqlx::query_file!("tests/task_item_artifacts/completed_intents.sql", object_id)
        .fetch_one(db.pool())
        .await
        .expect("count completed intents")
        .count
        .expect("completed intent count")
}

async fn latest_intent_error(db: &Database, object_id: Uuid) -> Option<String> {
    sqlx::query_file!(
        "tests/task_item_artifacts/latest_intent_error.sql",
        object_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read latest intent error")
    .last_error
}

async fn force_intent_due(db: &Database, object_id: Uuid) {
    sqlx::query_file!("tests/task_item_artifacts/force_intent_due.sql", object_id)
        .execute(db.pool())
        .await
        .expect("force intent due");
}

async fn cleanup(db: &Database, groups: &[i64], user_id: i64) {
    sqlx::query_file!(
        "tests/task_item_artifacts/delete_tasks_in_groups.sql",
        groups
    )
    .execute(db.pool())
    .await
    .expect("clean up tasks");
    sqlx::query_file!(
        "tests/task_item_artifacts/delete_files_in_groups.sql",
        groups
    )
    .execute(db.pool())
    .await
    .expect("clean up files");
    sqlx::query_file!(
        "tests/task_item_artifacts/delete_intents_in_groups.sql",
        groups
    )
    .execute(db.pool())
    .await
    .expect("clean up intents");
    sqlx::query_file!(
        "tests/task_item_artifacts/delete_objects_in_groups.sql",
        groups
    )
    .execute(db.pool())
    .await
    .expect("clean up objects");
    sqlx::query_file!("tests/task_item_artifacts/delete_groups.sql", groups)
        .execute(db.pool())
        .await
        .expect("clean up groups");
    sqlx::query_file!("tests/task_item_artifacts/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

fn storage_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("context69-task-artifact-{}", Uuid::new_v4()))
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_artifact_only_reference_survives_both_guarded_delete_paths_and_is_counted() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, _) = seed_object(&db, &root, owner.group_id, b"artifact-only bytes").await;
    seed_artifact(&db, item, KIND_DOWNLOAD, object).await;

    let store = LibraryStore::new(db.clone());
    assert_eq!(
        store
            .count_storage_object_references(object)
            .await
            .expect("count references"),
        1,
        "the artifact reference is the only reference and must be counted"
    );
    assert!(
        store
            .delete_unreferenced_storage_object(object)
            .await
            .expect("guarded unreferenced delete")
            .is_none(),
        "the guarded unreferenced delete must skip a referenced object"
    );
    let mut tx = db.pool().begin().await.expect("begin delete transaction");
    assert!(
        store
            .delete_cleanup_storage_object(&mut tx, object)
            .await
            .expect("guarded cleanup delete")
            .is_none(),
        "the cleanup worker's final row delete must skip a referenced object"
    );
    tx.rollback().await.expect("release delete transaction");
    assert_eq!(object_rows(&db, object).await, 1);
    assert!(root.join(&key).exists(), "physical bytes stay on disk");

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_artifact_reference_blocks_the_staged_sweep_and_staging_lease_clear() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, sha) = seed_object(&db, &root, owner.group_id, b"staged artifact").await;
    seed_artifact(&db, item, KIND_SECTIONS, object).await;
    seed_file_reference(&db, owner.group_id, object, &key, &sha).await;
    let expired = (Utc::now() - ChronoDuration::hours(1)).trunc_subsecs(6);
    set_staging_lease(&db, object, Some(expired)).await;

    let store = LibraryStore::new(db.clone());
    assert!(
        store
            .sweep_orphaned_storage_objects(Utc::now(), 10)
            .await
            .expect("list staged orphans")
            .is_empty(),
        "an artifact-owned staged object is never an orphan candidate"
    );
    assert_eq!(
        store
            .clear_expired_staging_with_file_reference(Utc::now(), 10)
            .await
            .expect("clear expired staging lease"),
        0,
        "an artifact-owned staging lease must not be cleared early"
    );
    assert_eq!(staging_lease(&db, object).await, Some(expired));

    sqlx::query_file!(
        "tests/task_item_artifacts/delete_artifact.sql",
        item,
        KIND_SECTIONS
    )
    .execute(db.pool())
    .await
    .expect("delete the artifact reference");
    assert_eq!(
        open_intents(&db, object).await,
        0,
        "a still-staged old object stays with the existing staging sweep"
    );
    assert_eq!(
        store
            .clear_expired_staging_with_file_reference(Utc::now(), 10)
            .await
            .expect("clear released staging lease"),
        1
    );
    assert_eq!(
        staging_lease(&db, object).await,
        None,
        "the lease clears once no artifact owns the object"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn file_task_input_and_artifact_references_share_one_object_safely() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, sha) = seed_object(&db, &root, owner.group_id, b"shared bytes").await;
    seed_file_reference(&db, owner.group_id, object, &key, &sha).await;
    seed_task_input_reference(&db, item, object).await;
    seed_artifact(&db, item, KIND_DOWNLOAD, object).await;

    let store = LibraryStore::new(db.clone());
    assert_eq!(
        store
            .count_storage_object_references(object)
            .await
            .expect("count references"),
        3,
        "file, task input and artifact references all count"
    );
    assert!(
        store
            .delete_unreferenced_storage_object(object)
            .await
            .expect("guarded unreferenced delete")
            .is_none()
    );
    let mut tx = db.pool().begin().await.expect("begin delete transaction");
    assert!(
        store
            .delete_cleanup_storage_object(&mut tx, object)
            .await
            .expect("guarded cleanup delete")
            .is_none()
    );
    tx.rollback().await.expect("release delete transaction");
    assert_eq!(object_rows(&db, object).await, 1);
    assert!(root.join(&key).exists());

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn deleting_artifacts_enqueues_the_exact_old_identity_and_dedupes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, sha) = seed_object(&db, &root, owner.group_id, b"dedupe").await;
    seed_artifact(&db, item, KIND_DOWNLOAD, object).await;
    seed_artifact(&db, item, KIND_SECTIONS, object).await;

    for kind in [KIND_DOWNLOAD, KIND_SECTIONS] {
        sqlx::query_file!("tests/task_item_artifacts/delete_artifact.sql", item, kind)
            .execute(db.pool())
            .await
            .expect("delete artifact reference");
    }

    assert_eq!(
        open_intents(&db, object).await,
        1,
        "two references to one object yield exactly one durable intent"
    );
    let identity = sqlx::query_file!("tests/task_item_artifacts/open_intent_identity.sql", object)
        .fetch_one(db.pool())
        .await
        .expect("read intent identity");
    assert_eq!(identity.group_id, owner.group_id);
    assert_eq!(identity.sha256.as_deref(), Some(sha.as_str()));
    assert_eq!(identity.object_key, key);
    assert_eq!(identity.storage_backend, "local");

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_retarget_enqueues_only_the_replaced_object() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (first, first_key, first_sha) = seed_object(&db, &root, owner.group_id, b"first").await;
    let (second, second_key, second_sha) = seed_object(&db, &root, owner.group_id, b"second").await;
    seed_artifact(&db, item, KIND_DOWNLOAD, first).await;

    sqlx::query_file!(
        "tests/task_item_artifacts/touch_artifact_same_object.sql",
        item,
        KIND_DOWNLOAD
    )
    .execute(db.pool())
    .await
    .expect("same-object update");
    assert_eq!(
        open_intents(&db, first).await,
        0,
        "an update that keeps the same object is not a release"
    );

    sqlx::query_file!(
        "tests/task_item_artifacts/retarget_artifact.sql",
        item,
        KIND_DOWNLOAD,
        second
    )
    .execute(db.pool())
    .await
    .expect("retarget artifact");
    assert_eq!(open_intents(&db, first).await, 1);
    assert_eq!(open_intents(&db, second).await, 0);
    let replaced = sqlx::query_file!("tests/task_item_artifacts/open_intent_identity.sql", first)
        .fetch_one(db.pool())
        .await
        .expect("read replaced identity");
    assert_eq!(replaced.object_key, first_key);
    assert_eq!(replaced.sha256.as_deref(), Some(first_sha.as_str()));

    sqlx::query_file!(
        "tests/task_item_artifacts/delete_artifact.sql",
        item,
        KIND_DOWNLOAD
    )
    .execute(db.pool())
    .await
    .expect("delete artifact");
    assert_eq!(open_intents(&db, second).await, 1);
    let released = sqlx::query_file!("tests/task_item_artifacts/open_intent_identity.sql", second)
        .fetch_one(db.pool())
        .await
        .expect("read released identity");
    assert_eq!(released.object_key, second_key);
    assert_eq!(released.sha256.as_deref(), Some(second_sha.as_str()));

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_task_cascade_enqueues_the_old_object_and_rolls_back_with_the_delete() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task, item) = seed_task_item(&db, &owner).await;
    let (object, _, _) = seed_object(&db, &root, owner.group_id, b"cascade").await;
    seed_artifact(&db, item, KIND_DOWNLOAD, object).await;

    let mut tx = db.pool().begin().await.expect("begin cascade transaction");
    sqlx::query_file!("tests/task_item_artifacts/delete_task.sql", task)
        .execute(&mut *tx)
        .await
        .expect("delete task inside the transaction");
    let inside_tx: i64 = sqlx::query_file!("tests/task_item_artifacts/open_intents.sql", object)
        .fetch_one(&mut *tx)
        .await
        .expect("read intent inside the transaction")
        .count
        .expect("intent count");
    assert_eq!(inside_tx, 1, "the cascade fires the release hook");
    tx.rollback().await.expect("roll back the cascade");

    assert_eq!(
        open_intents(&db, object).await,
        0,
        "the intent rolls back with the reference mutation"
    );
    assert_eq!(
        sqlx::query_file!("tests/task_item_artifacts/open_intents.sql", object)
            .fetch_one(db.pool())
            .await
            .expect("read committed intent")
            .count
            .expect("intent count"),
        0
    );

    sqlx::query_file!("tests/task_item_artifacts/delete_task.sql", task)
        .execute(db.pool())
        .await
        .expect("delete the task");
    assert_eq!(open_intents(&db, object).await, 1);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_staged_object_deletion_defers_to_the_existing_staging_sweep() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, _, _) = seed_object(&db, &root, owner.group_id, b"staged release").await;
    seed_artifact(&db, item, KIND_SECTIONS, object).await;
    set_staging_lease(&db, object, Some(Utc::now() + ChronoDuration::hours(1))).await;

    sqlx::query_file!(
        "tests/task_item_artifacts/delete_artifact.sql",
        item,
        KIND_SECTIONS
    )
    .execute(db.pool())
    .await
    .expect("delete the staged artifact reference");
    assert_eq!(
        open_intents(&db, object).await,
        0,
        "a staged object has no release intent; the staging sweep owns it"
    );
    assert_eq!(object_rows(&db, object).await, 1);

    let store = LibraryStore::new(db.clone());
    assert!(
        store
            .sweep_orphaned_storage_objects(Utc::now(), 10)
            .await
            .expect("list staged orphans")
            .is_empty(),
        "an unexpired staging lease is not yet reclaimable"
    );
    set_staging_lease(&db, object, Some(Utc::now() - ChronoDuration::hours(1))).await;
    let candidates = store
        .sweep_orphaned_storage_objects(Utc::now(), 10)
        .await
        .expect("list expired staged orphans");
    assert_eq!(
        candidates.len(),
        1,
        "the expired staged object is reclaimable"
    );
    assert_eq!(candidates[0].id, object);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_cleanup_worker_retains_bytes_until_the_last_artifact_reference_is_gone() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, sha) = seed_object(&db, &root, owner.group_id, b"worker bytes").await;
    seed_artifact(&db, item, KIND_DOWNLOAD, object).await;
    let service = build_service(&db, &root).await;

    // Model a stale or concurrent release intent: only the reference count may
    // keep the bytes.
    sqlx::query_file!(
        "tests/task_item_artifacts/insert_cleanup_intent.sql",
        object,
        owner.group_id,
        sha.clone(),
        key.clone(),
        "local"
    )
    .execute(db.pool())
    .await
    .expect("insert release intent");
    force_intent_due(&db, object).await;
    let retained = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup pass");
    assert!(
        retained.cancelled >= 1,
        "the worker must cancel while an artifact reference remains"
    );
    assert_eq!(object_rows(&db, object).await, 1);
    assert!(
        root.join(&key).exists(),
        "referenced bytes are never deleted"
    );
    assert_eq!(open_intents(&db, object).await, 0);
    assert_eq!(completed_intents(&db, object).await, 1);
    assert!(
        latest_intent_error(&db, object)
            .await
            .unwrap_or_default()
            .contains("referenced again")
    );

    sqlx::query_file!(
        "tests/task_item_artifacts/delete_artifact.sql",
        item,
        KIND_DOWNLOAD
    )
    .execute(db.pool())
    .await
    .expect("delete the last artifact reference");
    assert_eq!(open_intents(&db, object).await, 1);
    force_intent_due(&db, object).await;
    let released = service
        .run_source_object_cleanup(50)
        .await
        .expect("release cleanup pass");
    assert!(released.deleted >= 1);
    assert_eq!(object_rows(&db, object).await, 0);
    assert!(
        !root.join(&key).exists(),
        "the last reference releases bytes"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_release_identity_comes_from_the_registry_not_the_task_group() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let object_owner = seed_owner(&db).await;
    let task_owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &task_owner).await;
    let (object, key, sha) = seed_object(&db, &root, object_owner.group_id, b"cross-group").await;
    seed_artifact(&db, item, KIND_DOWNLOAD, object).await;

    sqlx::query_file!(
        "tests/task_item_artifacts/delete_artifact.sql",
        item,
        KIND_DOWNLOAD
    )
    .execute(db.pool())
    .await
    .expect("delete the cross-group artifact reference");
    let identity = sqlx::query_file!("tests/task_item_artifacts/open_intent_identity.sql", object)
        .fetch_one(db.pool())
        .await
        .expect("read intent identity");
    assert_eq!(
        identity.group_id, object_owner.group_id,
        "the intent carries the object's group, never the task's"
    );
    assert_eq!(identity.object_key, key);
    assert_eq!(identity.sha256.as_deref(), Some(sha.as_str()));

    let service = build_service(&db, &root).await;
    force_intent_due(&db, object).await;
    let released = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup cross-group release");
    assert!(released.deleted >= 1);
    assert!(!root.join(&key).exists());

    cleanup(
        &db,
        &[object_owner.group_id, task_owner.group_id],
        object_owner.user_id,
    )
    .await;
    cleanup(&db, &[], task_owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_mismatched_intent_never_deletes_another_groups_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let foreign = seed_owner(&db).await;
    let (object, key, sha) = seed_object(&db, &root, owner.group_id, b"protected").await;

    sqlx::query_file!(
        "tests/task_item_artifacts/insert_cleanup_intent.sql",
        object,
        foreign.group_id,
        sha,
        key.clone(),
        "local"
    )
    .execute(db.pool())
    .await
    .expect("insert mismatched intent");
    force_intent_due(&db, object).await;
    let service = build_service(&db, &root).await;
    let summary = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup mismatched intent");
    assert!(summary.cancelled >= 1, "an identity mismatch is cancelled");
    assert_eq!(object_rows(&db, object).await, 1);
    assert!(
        root.join(&key).exists(),
        "a foreign intent never deletes another group's bytes"
    );
    assert!(
        latest_intent_error(&db, object)
            .await
            .unwrap_or_default()
            .contains("identity changed")
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    cleanup(&db, &[foreign.group_id], foreign.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_new_artifact_reference_waits_for_the_cleanup_identity_lock() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, _) = seed_object(&db, &root, owner.group_id, b"lock race").await;
    let store = LibraryStore::new(db.clone());

    let mut lock_tx = db.pool().begin().await.expect("begin lock transaction");
    store
        .lock_cleanup_storage_object(&mut lock_tx, object)
        .await
        .expect("lock object")
        .expect("object exists");

    let insert = tokio::spawn({
        let db = db.clone();
        async move { seed_artifact(&db, item, KIND_DOWNLOAD, object).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(
        !insert.is_finished(),
        "a new artifact reference must wait for the cleanup identity lock"
    );
    lock_tx.rollback().await.expect("release object lock");
    insert.await.expect("join artifact insert");

    assert_eq!(
        store
            .count_storage_object_references(object)
            .await
            .expect("count references"),
        1
    );
    assert!(root.join(&key).exists());

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

async fn seed_running_item(
    db: &Database,
    owner: &Owner,
    payload: serde_json::Value,
    lease_token: Uuid,
) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_task.sql",
        task_id,
        owner.user_id,
        owner.group_id
    )
    .execute(db.pool())
    .await
    .expect("seed task");
    let item_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_running_item.sql",
        item_id,
        task_id,
        payload,
        lease_token
    )
    .execute(db.pool())
    .await
    .expect("seed running item");
    (task_id, item_id)
}

async fn item_payload(db: &Database, item_id: Uuid) -> serde_json::Value {
    sqlx::query_file!("tests/task_item_artifacts/get_item_payload.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read item payload")
        .payload
}

async fn artifact_reference(
    db: &Database,
    item_id: Uuid,
    kind: &str,
) -> Option<(Uuid, String, String)> {
    sqlx::query_file!(
        "tests/task_item_artifacts/get_artifact_reference.sql",
        item_id,
        kind
    )
    .fetch_optional(db.pool())
    .await
    .expect("read artifact reference")
    .map(|row| (row.storage_object_id, row.kind, row.encoding))
}

async fn artifact_count(db: &Database, item_id: Uuid) -> i64 {
    sqlx::query_file!("tests/task_item_artifacts/count_artifacts.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("count artifact references")
        .count
        .expect("artifact count")
}

async fn object_key(db: &Database, object_id: Uuid) -> String {
    sqlx::query_file!("tests/task_item_artifacts/get_object_key.sql", object_id)
        .fetch_one(db.pool())
        .await
        .expect("read object key")
        .object_key
}

fn download_payload(_content: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "url": "https://example.test/document.pdf",
        "filename": "document.pdf",
        "download_artifact": {
            "content_base64": "aW5saW5l",
            "url": "https://example.test/document.pdf",
            "filename": "document.pdf",
            "media_type": "application/pdf",
        },
        "unrelated": {"kept": true},
    })
}

/// Count files below the staging prefix (directories alone are not bytes).
fn staging_file_count(root: &Path) -> usize {
    fn walk(path: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        entries
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() { walk(&path) } else { 1 }
            })
            .sum()
    }
    walk(&root.join("staging"))
}

fn sections_payload() -> serde_json::Value {
    serde_json::json!({
        "section_payload": [{"order": 0, "text": "section body"}],
        "indexing_checkpoint": {"completed_chunks": 3},
        "file_id": "kept",
    })
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn staging_an_artifact_registers_verified_content_addressed_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"payload"), lease).await;
    let service = build_service(&db, &root).await;

    let content = Bytes::from_static(b"artifact bytes that must round-trip");
    let staged = service
        .stage_task_item_artifact(owner.group_id, content.clone(), lease)
        .await
        .expect("stage artifact");

    assert_eq!(staged.group_id, owner.group_id);
    assert_eq!(staged.size_bytes, content.len() as i64);
    let key = object_key(&db, staged.object_id).await;
    assert_eq!(key, format!("objects/{}/{}", owner.group_id, staged.sha256));
    assert_eq!(
        std::fs::read(root.join(&key)).expect("read object"),
        content
    );
    assert!(
        staging_lease(&db, staged.object_id).await.is_some(),
        "a staged artifact keeps its 24-hour provisional lease"
    );
    assert_eq!(artifact_count(&db, item).await, 0);
    assert_eq!(
        service
            .read_task_item_artifact(item, TaskItemArtifactKind::Download, 1024)
            .await
            .expect("read without a reference"),
        None
    );
    assert_eq!(
        staging_file_count(&root),
        0,
        "the temporary staging key is removed after finalization"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn attaching_download_strips_only_the_inline_bytes_and_keeps_the_identity() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let content = b"download bytes";
    let (_, item) = seed_running_item(&db, &owner, download_payload(content), lease).await;
    let service = build_service(&db, &root).await;

    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::copy_from_slice(content), lease)
        .await
        .expect("stage artifact");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach artifact"),
        "the running lease may attach"
    );

    let payload = item_payload(&db, item).await;
    let artifact = payload
        .get("download_artifact")
        .expect("the artifact identity stays inline");
    assert!(
        artifact.get("content_base64").is_none(),
        "only the migrated inline bytes are removed"
    );
    assert_eq!(artifact["url"], "https://example.test/document.pdf");
    assert_eq!(artifact["filename"], "document.pdf");
    assert_eq!(artifact["media_type"], "application/pdf");
    assert_eq!(payload["url"], "https://example.test/document.pdf");
    assert_eq!(payload["unrelated"], serde_json::json!({"kept": true}));

    let reference = artifact_reference(&db, item, "download")
        .await
        .expect("download reference exists");
    assert_eq!(reference.0, staged.object_id);
    assert_eq!(reference.1, "download");
    assert_eq!(reference.2, "raw-v1");
    assert_eq!(
        staging_lease(&db, staged.object_id).await,
        None,
        "a committed reference promotes the object"
    );

    let read = service
        .read_task_item_artifact(item, TaskItemArtifactKind::Download, 1024)
        .await
        .expect("read the attached artifact")
        .expect("the artifact exists");
    assert_eq!(read.as_ref(), content);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn attaching_sections_preserves_the_indexing_checkpoint_and_other_keys() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, sections_payload(), lease).await;
    let service = build_service(&db, &root).await;

    let staged = service
        .stage_task_item_artifact(
            owner.group_id,
            Bytes::from_static(b"[{\"order\":0}]"),
            lease,
        )
        .await
        .expect("stage artifact");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Sections, &staged)
            .await
            .expect("attach sections")
    );

    let payload = item_payload(&db, item).await;
    assert!(payload.get("section_payload").is_none());
    assert_eq!(
        payload["indexing_checkpoint"],
        serde_json::json!({"completed_chunks": 3})
    );
    assert_eq!(payload["file_id"], "kept");
    let reference = artifact_reference(&db, item, "sections")
        .await
        .expect("sections reference exists");
    assert_eq!(reference.2, "ingest-sections-json-v1");

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_item_without_inline_bytes_still_attaches_and_keeps_its_payload() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let payload = serde_json::json!({"url": "https://example.test/external"});
    let (_, item) = seed_running_item(&db, &owner, payload.clone(), lease).await;
    let service = build_service(&db, &root).await;

    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"bytes"), lease)
        .await
        .expect("stage artifact");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach without an inline value")
    );
    assert_eq!(item_payload(&db, item).await, payload);
    assert_eq!(artifact_count(&db, item).await, 1);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lost_lease_or_changed_status_never_strips_or_references() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"retry me"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"retry me"), lease)
        .await
        .expect("stage artifact");

    sqlx::query_file!(
        "tests/task_item_artifacts/change_item_lease.sql",
        item,
        Uuid::new_v4()
    )
    .execute(db.pool())
    .await
    .expect("replace the lease");
    assert!(
        !service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach with a stale lease"),
        "a reclaim loses the attach"
    );
    assert!(
        item_payload(&db, item)
            .await
            .pointer("/download_artifact/content_base64")
            .is_some(),
        "the inline retry input survives"
    );
    assert_eq!(artifact_count(&db, item).await, 0);
    assert_eq!(
        object_rows(&db, staged.object_id).await,
        0,
        "the unreferenced provisional object is released"
    );

    sqlx::query_file!(
        "tests/task_item_artifacts/set_item_status.sql",
        item,
        "cancelled"
    )
    .execute(db.pool())
    .await
    .expect("cancel the item");
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"retry me"), lease)
        .await
        .expect("stage another artifact");
    assert!(
        !service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach on a cancelled item")
    );
    assert_eq!(artifact_count(&db, item).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_concurrent_reclaim_while_attach_waits_leaves_the_payload_untouched() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"raced"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"raced"), lease)
        .await
        .expect("stage artifact");

    let mut lock = db.pool().begin().await.expect("begin item lock");
    sqlx::query_file!("tests/task_item_artifacts/lock_item_row.sql", item)
        .fetch_one(&mut *lock)
        .await
        .expect("lock the item row");
    let attach = tokio::spawn({
        let service = service.clone();
        let staged = staged.clone();
        async move {
            service
                .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
                .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !attach.is_finished(),
        "attach must wait for the item row lock"
    );
    sqlx::query_file!(
        "tests/task_item_artifacts/change_item_lease.sql",
        item,
        Uuid::new_v4()
    )
    .execute(&mut *lock)
    .await
    .expect("reclaim the lease inside the lock");
    lock.commit().await.expect("commit the reclaim");

    assert!(
        !attach
            .await
            .expect("join attach")
            .expect("attach after the reclaim"),
        "the statement-time lease guard rejects a reclaimed item"
    );
    assert!(
        item_payload(&db, item)
            .await
            .pointer("/download_artifact/content_base64")
            .is_some()
    );
    assert_eq!(artifact_count(&db, item).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_cleanup_identity_lock_blocks_attach_until_the_reference_can_commit() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"locked"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"locked"), lease)
        .await
        .expect("stage artifact");

    let store = LibraryStore::new(db.clone());
    let mut cleanup_tx = db.pool().begin().await.expect("begin cleanup lock");
    store
        .lock_cleanup_storage_object(&mut cleanup_tx, staged.object_id)
        .await
        .expect("lock the object identity")
        .expect("object exists");
    let attach = tokio::spawn({
        let service = service.clone();
        let staged = staged.clone();
        async move {
            service
                .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
                .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !attach.is_finished(),
        "the reference insert must wait for the cleanup identity lock"
    );
    cleanup_tx
        .rollback()
        .await
        .expect("release the identity lock");
    assert!(
        attach
            .await
            .expect("join attach")
            .expect("attach after cleanup released the lock")
    );
    assert_eq!(artifact_count(&db, item).await, 1);
    assert!(
        item_payload(&db, item)
            .await
            .pointer("/download_artifact/content_base64")
            .is_none()
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_wrong_group_object_is_refused_without_touching_the_payload() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let foreign = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"cross group"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(foreign.group_id, Bytes::from_static(b"cross group"), lease)
        .await
        .expect("stage in another group");

    assert!(
        !service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach a foreign object"),
        "the task group must own the registry object"
    );
    assert!(
        item_payload(&db, item)
            .await
            .pointer("/download_artifact/content_base64")
            .is_some()
    );
    assert_eq!(artifact_count(&db, item).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    cleanup(&db, &[foreign.group_id], foreign.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn corrupt_or_missing_bytes_never_strip_the_inline_value() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"good bytes"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"good bytes"), lease)
        .await
        .expect("stage artifact");
    let key = object_key(&db, staged.object_id).await;

    std::fs::write(root.join(&key), b"corrupted!!!").expect("corrupt the object");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .is_err(),
        "attach must fail closed on an integrity mismatch"
    );
    assert!(
        item_payload(&db, item)
            .await
            .pointer("/download_artifact/content_base64")
            .is_some()
    );
    assert_eq!(artifact_count(&db, item).await, 0);
    set_staging_lease(
        &db,
        staged.object_id,
        Some(Utc::now() - ChronoDuration::hours(1)),
    )
    .await;
    let store = LibraryStore::new(db.clone());
    let candidates = store
        .sweep_orphaned_storage_objects(Utc::now(), 10)
        .await
        .expect("list expired provisional objects");
    assert!(
        candidates
            .iter()
            .any(|object| object.id == staged.object_id),
        "a failed provisional object stays eligible for the existing staging sweep"
    );

    std::fs::remove_file(root.join(&key)).expect("remove the object bytes");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .is_err(),
        "attach must fail closed on a missing object"
    );
    assert!(
        item_payload(&db, item)
            .await
            .pointer("/download_artifact/content_base64")
            .is_some()
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_read_cap_is_enforced_before_and_during_the_stream() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let content = vec![b'x'; 4096];
    let (_, item) = seed_running_item(&db, &owner, download_payload(&content), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from(content.clone()), lease)
        .await
        .expect("stage artifact");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach artifact")
    );

    assert!(
        service
            .read_task_item_artifact(item, TaskItemArtifactKind::Download, 1024)
            .await
            .is_err(),
        "the registry size is rejected before any read"
    );

    sqlx::query_file!(
        "tests/task_item_artifacts/set_object_size.sql",
        staged.object_id,
        16_i64
    )
    .execute(db.pool())
    .await
    .expect("understate the registry size");
    assert!(
        service
            .read_task_item_artifact(item, TaskItemArtifactKind::Download, 1024)
            .await
            .is_err(),
        "the streamed cap and digest refuse understated metadata"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_retarget_releases_only_the_replaced_object_and_a_repeat_is_a_noop() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"first"), lease).await;
    let service = build_service(&db, &root).await;
    let first = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"first"), lease)
        .await
        .expect("stage first");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &first)
            .await
            .expect("attach first")
    );
    let second = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"second"), lease)
        .await
        .expect("stage second");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &second)
            .await
            .expect("retarget")
    );
    assert_eq!(open_intents(&db, first.object_id).await, 1);
    assert_eq!(open_intents(&db, second.object_id).await, 0);
    assert_eq!(
        artifact_reference(&db, item, "download")
            .await
            .expect("reference")
            .0,
        second.object_id
    );

    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &second)
            .await
            .expect("repeat the same attach")
    );
    assert_eq!(
        open_intents(&db, second.object_id).await,
        0,
        "a still-referenced object has no release intent"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn promotion_requires_a_reference_and_preserves_a_shared_input_lease() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"shared"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"shared"), lease)
        .await
        .expect("stage artifact");

    let store = LibraryStore::new(db.clone());
    assert!(
        !store
            .promote_task_item_artifact_object(staged.object_id)
            .await
            .expect("promote without a reference"),
        "a provisional object is never promoted"
    );
    assert!(staging_lease(&db, staged.object_id).await.is_some());

    seed_task_input_reference(&db, item, staged.object_id).await;
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach with a shared input")
    );
    assert!(
        staging_lease(&db, staged.object_id).await.is_some(),
        "a shared task input keeps its staging lease"
    );

    sqlx::query_file!(
        "tests/task_item_artifacts/clear_task_input_reference.sql",
        item
    )
    .execute(db.pool())
    .await
    .expect("release the shared input reference");
    assert!(
        store
            .promote_task_item_artifact_object(staged.object_id)
            .await
            .expect("promote after the input reference is gone")
    );
    assert_eq!(staging_lease(&db, staged.object_id).await, None);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn both_kinds_coexist_on_one_item_and_read_back_verified() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let payload = serde_json::json!({
        "download_artifact": {"content_base64": "AAAA", "filename": "a.pdf"},
        "section_payload": [{"text": "body"}],
        "indexing_checkpoint": {"completed_chunks": 1},
    });
    let (_, item) = seed_running_item(&db, &owner, payload, lease).await;
    let service = build_service(&db, &root).await;
    let download = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"download"), lease)
        .await
        .expect("stage download");
    let sections = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"sections"), lease)
        .await
        .expect("stage sections");

    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &download)
            .await
            .expect("attach download")
    );
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Sections, &sections)
            .await
            .expect("attach sections")
    );

    let payload = item_payload(&db, item).await;
    assert!(
        payload
            .pointer("/download_artifact/content_base64")
            .is_none()
    );
    assert!(payload.get("section_payload").is_none());
    assert_eq!(payload["download_artifact"]["filename"], "a.pdf");
    assert_eq!(
        payload["indexing_checkpoint"],
        serde_json::json!({"completed_chunks": 1})
    );
    assert_eq!(artifact_count(&db, item).await, 2);
    assert_eq!(
        service
            .read_task_item_artifact(item, TaskItemArtifactKind::Download, 1024)
            .await
            .expect("read download")
            .expect("download exists")
            .as_ref(),
        b"download"
    );
    assert_eq!(
        service
            .read_task_item_artifact(item, TaskItemArtifactKind::Sections, 1024)
            .await
            .expect("read sections")
            .expect("sections exist")
            .as_ref(),
        b"sections"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_database_guards_refuse_a_wrong_digest_size_or_group() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let other = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"guards"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"guards"), lease)
        .await
        .expect("stage artifact");

    let base = context69::db::AttachTaskItemArtifactRequest {
        item_id: item,
        lease_token: lease,
        kind: TaskItemArtifactKind::Download,
        storage_object_id: staged.object_id,
        sha256: &staged.sha256,
        size_bytes: staged.size_bytes,
        storage_backend: "local",
    };
    let wrong_digest = "0".repeat(64);
    let wrong_size = staged.size_bytes + 1;
    for request in [
        context69::db::AttachTaskItemArtifactRequest {
            sha256: &wrong_digest,
            ..base
        },
        context69::db::AttachTaskItemArtifactRequest {
            size_bytes: wrong_size,
            ..base
        },
        context69::db::AttachTaskItemArtifactRequest {
            storage_backend: "s3",
            ..base
        },
    ] {
        assert!(
            !db.attach_task_item_artifact(request)
                .await
                .expect("guarded database attach"),
            "a mismatched registry identity must be refused"
        );
        assert_eq!(artifact_count(&db, item).await, 0);
        assert!(
            item_payload(&db, item)
                .await
                .pointer("/download_artifact/content_base64")
                .is_some()
        );
    }

    let other_lease = Uuid::new_v4();
    let (_, foreign_item) =
        seed_running_item(&db, &other, download_payload(b"foreign"), other_lease).await;
    assert!(
        !db.attach_task_item_artifact(context69::db::AttachTaskItemArtifactRequest {
            item_id: foreign_item,
            lease_token: other_lease,
            ..base
        })
        .await
        .expect("attach across groups"),
        "an item may only attach an object from its own task group"
    );
    assert_eq!(artifact_count(&db, foreign_item).await, 0);

    // The owning group still attaches; only the declared identity matters.
    assert!(
        db.attach_task_item_artifact(base)
            .await
            .expect("correct attach")
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    cleanup(&db, &[other.group_id], other.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn artifact_kind_pairs_are_fixed_and_unknown_pairs_are_refused() {
    assert_eq!(TaskItemArtifactKind::Download.kind(), "download");
    assert_eq!(TaskItemArtifactKind::Download.encoding(), "raw-v1");
    assert_eq!(TaskItemArtifactKind::Sections.kind(), "sections");
    assert_eq!(
        TaskItemArtifactKind::Sections.encoding(),
        "ingest-sections-json-v1"
    );
    assert_eq!(
        TaskItemArtifactKind::from_parts("download", "raw-v1"),
        Some(TaskItemArtifactKind::Download)
    );
    assert_eq!(
        TaskItemArtifactKind::from_parts("sections", "ingest-sections-json-v1"),
        Some(TaskItemArtifactKind::Sections)
    );
    for (kind, encoding) in [
        ("download", "ingest-sections-json-v1"),
        ("sections", "raw-v1"),
        ("download", "base64"),
        ("unknown", "raw-v1"),
        ("sections", ""),
    ] {
        assert_eq!(
            TaskItemArtifactKind::from_parts(kind, encoding),
            None,
            "unknown pair {kind}/{encoding} must be refused"
        );
    }
}

async fn task_exists(db: &Database, task_id: Uuid) -> bool {
    sqlx::query_file!("tests/task_item_artifacts/task_exists.sql", task_id)
        .fetch_one(db.pool())
        .await
        .expect("count task rows")
        .count
        .expect("task count")
        > 0
}

async fn item_count_for_task(db: &Database, task_id: Uuid) -> i64 {
    sqlx::query_file!("tests/task_item_artifacts/item_count_for_task.sql", task_id)
        .fetch_one(db.pool())
        .await
        .expect("count task items")
        .count
        .expect("item count")
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn rerun_copies_the_source_item_references_in_the_same_transaction() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (task_id, item) = seed_running_item(
        &db,
        &owner,
        serde_json::json!({"url": "https://example.test/rerun"}),
        lease,
    )
    .await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"rerun bytes"), lease)
        .await
        .expect("stage artifact");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach artifact")
    );

    let (new_task_id, new_items) = db.rerun_task(task_id).await.expect("rerun the task");
    assert_eq!(new_items.len(), 1);
    let reference = artifact_reference(&db, new_items[0], "download")
        .await
        .expect("the rerun item inherits the reference");
    assert_eq!(reference.0, staged.object_id);
    assert_eq!(reference.2, "raw-v1");
    assert_eq!(open_intents(&db, staged.object_id).await, 0);

    // Deleting the source task must not release an object the rerun still owns.
    sqlx::query_file!("tests/task_item_artifacts/delete_task.sql", task_id)
        .execute(db.pool())
        .await
        .expect("delete the source task");
    assert_eq!(object_rows(&db, staged.object_id).await, 1);
    assert_eq!(artifact_count(&db, new_items[0]).await, 1);

    // Retry/resume in place leave the reference untouched.
    sqlx::query_file!("tests/task_item_artifacts/delete_task.sql", new_task_id)
        .execute(db.pool())
        .await
        .expect("clean the rerun task");
    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_concurrent_source_task_delete_cannot_produce_a_partial_rerun() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (task_id, item) = seed_running_item(
        &db,
        &owner,
        serde_json::json!({"url": "https://example.test/race"}),
        lease,
    )
    .await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"race bytes"), lease)
        .await
        .expect("stage artifact");
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach artifact")
    );

    // Hold the source task row, start the rerun so it blocks on that lock, then
    // delete the source task and commit. The rerun must observe no source rows
    // instead of copying a reference whose bytes are already being released.
    let mut lock = db.pool().begin().await.expect("begin source lock");
    sqlx::query_file!("tests/task_item_artifacts/lock_task_row.sql", task_id)
        .fetch_one(&mut *lock)
        .await
        .expect("lock the source task");
    let rerun = tokio::spawn({
        let db = db.clone();
        async move { db.rerun_task(task_id).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !rerun.is_finished(),
        "rerun must wait for the source row lock"
    );
    sqlx::query_file!("tests/task_item_artifacts/delete_task.sql", task_id)
        .execute(&mut *lock)
        .await
        .expect("delete the source task inside the lock");
    lock.commit().await.expect("commit the source delete");

    let error = rerun
        .await
        .expect("join rerun")
        .expect_err("a deleted source task leaves nothing to rerun");
    assert!(
        error
            .to_string()
            .contains("rerun requires at least one item"),
        "the rerun must fail closed, got: {error}"
    );
    assert!(!task_exists(&db, task_id).await);
    assert_eq!(item_count_for_task(&db, task_id).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_shared_library_file_object_survives_an_artifact_release() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, serde_json::json!({}), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"shared life"), lease)
        .await
        .expect("stage artifact");
    let object = staged.object_id;
    let key = object_key(&db, object).await;
    seed_file_reference(&db, owner.group_id, object, &key, &staged.sha256).await;
    assert!(
        service
            .attach_task_item_artifact(item, lease, TaskItemArtifactKind::Download, &staged)
            .await
            .expect("attach artifact on a shared object")
    );
    assert_eq!(
        artifact_reference(&db, item, "download")
            .await
            .expect("reference")
            .0,
        object
    );

    sqlx::query_file!(
        "tests/task_item_artifacts/delete_artifact.sql",
        item,
        "download"
    )
    .execute(db.pool())
    .await
    .expect("release the artifact reference");
    force_intent_due(&db, object).await;
    let summary = service
        .run_source_object_cleanup(50)
        .await
        .expect("cleanup pass");
    assert!(summary.cancelled >= 1);
    assert_eq!(object_rows(&db, object).await, 1);
    assert!(root.join(&key).exists());
    assert!(
        latest_intent_error(&db, object)
            .await
            .unwrap_or_default()
            .contains("referenced again")
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

/// The artifact profile comes from the real admission result, never from a SQL
/// error: an ordinary migrated database must take the normal branch, where any
/// relation error propagates instead of being reported as an absent reference.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_ordinary_database_is_not_the_pinned_recovery_artifact_profile() {
    let (_guard, db) = prepare().await;
    assert!(
        db.pending_upgrade().is_none(),
        "a migrated database must never take the pre-artifact-schema recovery branch"
    );

    // The normal branch reads the artifact relation; a missing reference is
    // simply `None`, not an error, and real schema faults keep propagating.
    let service = build_service(&db, &storage_root()).await;
    let unknown = Uuid::new_v4();
    assert!(
        service
            .load_task_item_artifact(unknown, TaskItemArtifactKind::Sections)
            .await
            .expect("an item without a reference is not an error")
            .is_none()
    );
}

fn inline_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn backfill_args() -> context69::services::library::artifact_backfill::ArtifactBackfillArgs {
    context69::services::library::artifact_backfill::ArtifactBackfillArgs {
        apply: true,
        restore_inline: false,
        acknowledge_reader_upgrade: true,
        acknowledge_inline_growth: false,
        kind: context69::services::library::artifact_backfill::ArtifactKindFilter::All,
        item_ids: Vec::new(),
        page_size: 100,
        max_items: 500,
        max_artifact_bytes: 256 * 1024 * 1024,
        max_total_bytes: 512 * 1024 * 1024,
        delay_ms: 0,
        resume_cursor: None,
        help: false,
    }
}

fn preview_args() -> context69::services::library::artifact_backfill::ArtifactBackfillArgs {
    let mut args = backfill_args();
    args.apply = false;
    args.acknowledge_reader_upgrade = false;
    args
}

async fn make_terminal(db: &Database, task_id: Uuid, item_id: Uuid, payload: serde_json::Value) {
    sqlx::query_file!(
        "tests/task_item_artifacts/set_terminal_item.sql",
        item_id,
        payload,
        "succeeded"
    )
    .execute(db.pool())
    .await
    .expect("set terminal item");
    sqlx::query_file!(
        "tests/task_item_artifacts/set_task_terminal.sql",
        task_id,
        "succeeded"
    )
    .execute(db.pool())
    .await
    .expect("set terminal task");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn preview_reports_metadata_only_and_does_not_write() {
    let (_guard, db) = prepare().await;
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(b"hello"),
            "content_base64": "aGVsbG8="
        },
        "unrelated": {"kept": true}
    });
    make_terminal(&db, task_id, item_id, payload).await;

    let report = context69::services::library::artifact_backfill::run_preview(&db, &preview_args())
        .await
        .expect("preview runs");
    assert_eq!(report.mode, "preview");
    assert!(report.read_only);
    assert_eq!(report.artifact_schema, "present");
    assert_eq!(report.input_validation, "not_performed");
    assert!(report.eligible.download >= 1);
    assert_eq!(report.applied.download, 0);
    assert_eq!(artifact_count(&db, item_id).await, 0);
    assert!(
        artifact_reference(&db, item_id, KIND_DOWNLOAD)
            .await
            .is_none()
    );

    let payload_after = item_payload(&db, item_id).await;
    assert_eq!(
        payload_after["download_artifact"]["content_base64"], "aGVsbG8=",
        "preview must not remove inline bytes"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn terminal_apply_strips_only_the_verified_download_field_and_is_idempotent() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let decoded = b"hello";
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(decoded),
            "content_base64": base64::engine::general_purpose::STANDARD.encode(decoded)
        },
        "indexing_checkpoint": {"kept": true},
        "unrelated": {"kept": true}
    });
    make_terminal(&db, task_id, item_id, payload).await;
    let service = build_service(&db, &root).await;
    context69::services::library::artifact_backfill::validate_apply(&db)
        .await
        .expect("migration validation passes on the test database");

    let report =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &backfill_args())
            .await
            .expect("apply runs");
    assert!(
        report.error.is_none(),
        "unexpected error: {:?}",
        report.error
    );
    assert_eq!(report.applied.download, 1);
    assert_eq!(report.applied.sections, 0);

    let payload_after = item_payload(&db, item_id).await;
    assert!(
        payload_after["download_artifact"]
            .get("content_base64")
            .is_none(),
        "only the inline bytes are removed"
    );
    assert_eq!(
        payload_after["download_artifact"]["source_url"],
        "https://example.test/x"
    );
    assert_eq!(
        payload_after["download_artifact"]["sha256"],
        inline_sha256(decoded)
    );
    assert_eq!(payload_after["indexing_checkpoint"]["kept"], true);
    assert_eq!(payload_after["unrelated"]["kept"], true);

    let (object_id, kind, encoding) = artifact_reference(&db, item_id, KIND_DOWNLOAD)
        .await
        .expect("download artifact reference");
    assert_eq!(kind, KIND_DOWNLOAD);
    assert_eq!(encoding, ENCODING_DOWNLOAD);
    let stored =
        std::fs::read(root.join(object_key(&db, object_id).await)).expect("read stored artifact");
    assert_eq!(stored, decoded);
    let read_back = service
        .read_task_item_artifact(item_id, TaskItemArtifactKind::Download, 1024)
        .await
        .expect("read artifact")
        .expect("artifact exists");
    assert_eq!(read_back.as_ref(), decoded);

    let second =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &backfill_args())
            .await
            .expect("second apply runs");
    assert_eq!(second.applied.download, 0);
    assert_eq!(
        second
            .skipped
            .get("already_externalized")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(artifact_count(&db, item_id).await, 1);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn terminal_apply_leaves_inline_bytes_on_digest_mismatch() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(b"other"),
            "content_base64": "aGVsbG8="
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    let service = build_service(&db, &root).await;

    let report =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &backfill_args())
            .await
            .expect("apply runs");
    assert!(report.error.is_none());
    assert_eq!(report.applied.download, 0);
    assert_eq!(report.skipped.get("sha_mismatch").copied().unwrap_or(0), 1);
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(artifact_count(&db, item_id).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn preview_and_apply_skip_active_and_null_group_items() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(b"hello"),
            "content_base64": "aGVsbG8="
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    sqlx::query_file!(
        "tests/task_item_artifacts/change_item_lease.sql",
        item_id,
        Uuid::new_v4()
    )
    .execute(db.pool())
    .await
    .expect("leave a lease on a terminal item");

    let (null_task, null_item) = seed_task_item(&db, &owner).await;
    make_terminal(&db, null_task, null_item, payload.clone()).await;
    sqlx::query_file!(
        "tests/task_item_artifacts/set_task_group_null.sql",
        null_task
    )
    .execute(db.pool())
    .await
    .expect("remove the task group");

    let mut preview_args = preview_args();
    preview_args.kind =
        context69::services::library::artifact_backfill::ArtifactKindFilter::Download;
    let preview = context69::services::library::artifact_backfill::run_preview(&db, &preview_args)
        .await
        .expect("preview runs");
    assert!(preview.skipped.get("active_item").copied().unwrap_or(0) >= 1);
    assert!(preview.skipped.get("no_group").copied().unwrap_or(0) >= 1);

    let service = build_service(&db, &root).await;
    let mut apply_args = backfill_args();
    apply_args.kind = context69::services::library::artifact_backfill::ArtifactKindFilter::Download;
    let report =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &apply_args)
            .await
            .expect("apply runs");
    assert_eq!(report.applied.download, 0);
    assert!(report.skipped.get("active_item").copied().unwrap_or(0) >= 1);
    assert!(report.skipped.get("no_group").copied().unwrap_or(0) >= 1);
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(item_payload(&db, null_item).await, payload);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn terminal_apply_rejects_corrupt_sections_without_touching_inline() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let payload = serde_json::json!({
        "section_payload": [{"not": "an ingest section"}],
        "indexing_checkpoint": {"kept": true}
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    let service = build_service(&db, &root).await;

    let report =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &backfill_args())
            .await
            .expect("apply runs");
    assert!(report.error.is_none());
    assert_eq!(report.applied.sections, 0);
    assert_eq!(
        report.skipped.get("invalid_sections").copied().unwrap_or(0),
        1
    );
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(artifact_count(&db, item_id).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn terminal_apply_stops_on_storage_failure_without_touching_inline() {
    let (_guard, db) = prepare().await;
    let unusable_root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(b"hello"),
            "content_base64": "aGVsbG8="
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    let service = build_service(&db, &unusable_root).await;
    std::fs::remove_dir_all(&unusable_root).expect("remove storage root");
    std::fs::write(&unusable_root, b"not a directory").expect("replace storage root with a file");

    let report =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &backfill_args())
            .await
            .expect("apply reports a stopped batch");
    assert!(report.error.is_some(), "a storage failure stops the batch");
    assert!(!report.scan_complete);
    assert_eq!(report.applied.download, 0);
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(artifact_count(&db, item_id).await, 0);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_file(unusable_root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn operator_preview_pool_rejects_writes_by_construction() {
    let (_guard, _db) = prepare().await;
    let url = context69_test_support::accessor::database_url();
    let read_only = Database::connect_operator(&url, true)
        .await
        .expect("connect the read-only operator pool");
    let error = sqlx::query_file!("tests/task_item_artifacts/noop_write.sql")
        .execute(read_only.pool())
        .await
        .expect_err("a read-only pool must refuse a write statement");
    assert!(
        error.to_string().contains("read-only transaction"),
        "unexpected error: {error}"
    );
}

fn restore_args() -> context69::services::library::artifact_backfill::ArtifactBackfillArgs {
    let mut args = backfill_args();
    args.restore_inline = true;
    args.acknowledge_reader_upgrade = false;
    args.acknowledge_inline_growth = true;
    args
}

fn restore_preview_args() -> context69::services::library::artifact_backfill::ArtifactBackfillArgs {
    let mut args = preview_args();
    args.restore_inline = true;
    args
}

/// Seed a content-addressed object whose registry digest matches its bytes.
async fn seed_verified_object(
    db: &Database,
    storage_root: &Path,
    group_id: i64,
    content: &[u8],
) -> (Uuid, String) {
    let sha256 = inline_sha256(content);
    let object_id = Uuid::new_v4();
    let object_key = format!("objects/{group_id}/{sha256}");
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_storage_object.sql",
        object_id,
        group_id,
        sha256.clone(),
        content.len() as i64,
        object_key.clone()
    )
    .execute(db.pool())
    .await
    .expect("seed verified storage object");
    write_object(storage_root, &object_key, content);
    (object_id, sha256)
}

fn ingest_sections() -> serde_json::Value {
    serde_json::json!([{
        "section_key": "s1",
        "section_label": "Section 1",
        "title": "Title",
        "summary": null,
        "body_text": "Body text",
        "source_uri": null,
        "external_id": null,
        "published_at": null,
        "metadata_json": {"kept": true}
    }])
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn restore_roundtrips_both_kinds_after_forward_apply() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let service = build_service(&db, &root).await;

    let download_bytes = b"restored download bytes";
    let (download_task, download_item) = seed_task_item(&db, &owner).await;
    let download_payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(download_bytes),
            "content_base64": base64::engine::general_purpose::STANDARD.encode(download_bytes)
        },
        "indexing_checkpoint": {"kept": true},
        "unrelated": {"kept": true}
    });
    make_terminal(&db, download_task, download_item, download_payload.clone()).await;

    let (sections_task, sections_item) = seed_task_item(&db, &owner).await;
    let sections_payload = serde_json::json!({
        "section_payload": ingest_sections(),
        "indexing_checkpoint": {"kept": true},
        "unrelated": {"kept": true}
    });
    make_terminal(&db, sections_task, sections_item, sections_payload.clone()).await;

    let mut forward_args = backfill_args();
    forward_args.item_ids = vec![download_item, sections_item];
    let forward =
        context69::services::library::artifact_backfill::run_apply(&db, &service, &forward_args)
            .await
            .expect("forward apply runs");
    assert!(forward.error.is_none());
    assert_eq!(forward.applied.download, 1);
    assert_eq!(forward.applied.sections, 1);
    assert_eq!(artifact_count(&db, download_item).await, 1);
    assert_eq!(artifact_count(&db, sections_item).await, 1);

    let mut restore_preview = restore_preview_args();
    restore_preview.item_ids = vec![download_item, sections_item];
    let preview = context69::services::library::artifact_backfill::run_restore_preview(
        &db,
        &restore_preview,
        "local",
    )
    .await
    .expect("restore preview runs");
    assert_eq!(preview.direction, "restore_inline");
    assert!(preview.read_only);
    assert_eq!(preview.input_validation, "not_performed");
    assert_eq!(preview.eligible.download, 1);
    assert_eq!(preview.eligible.sections, 1);
    assert_eq!(preview.restored.download, 0);
    assert_eq!(
        artifact_count(&db, download_item).await,
        1,
        "preview must not remove references"
    );

    let mut restore_apply = restore_args();
    restore_apply.item_ids = vec![download_item, sections_item];
    let restore = context69::services::library::artifact_backfill::run_restore_apply(
        &db,
        &service,
        &restore_apply,
    )
    .await
    .expect("restore apply runs");
    assert!(
        restore.error.is_none(),
        "unexpected error: {:?}",
        restore.error
    );
    assert_eq!(restore.restored.download, 1);
    assert_eq!(restore.restored.sections, 1);
    assert!(restore.remaining_artifact_references.is_some());

    assert_eq!(item_payload(&db, download_item).await, download_payload);
    assert_eq!(item_payload(&db, sections_item).await, sections_payload);
    assert_eq!(artifact_count(&db, download_item).await, 0);
    assert_eq!(artifact_count(&db, sections_item).await, 0);

    let repeat = context69::services::library::artifact_backfill::run_restore_apply(
        &db,
        &service,
        &restore_apply,
    )
    .await
    .expect("repeat restore runs");
    assert_eq!(repeat.restored.download, 0);
    assert_eq!(repeat.restored.sections, 0);
    assert_eq!(repeat.scanned, 0, "no artifact references remain");

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn restore_leaves_inline_and_reference_untouched_on_corrupt_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let content = b"corrupt me";
    let (object_id, sha256) = seed_verified_object(&db, &root, owner.group_id, content).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": sha256
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    seed_artifact(&db, item_id, KIND_DOWNLOAD, object_id).await;
    write_object(&root, &object_key(&db, object_id).await, b"other bytes");
    let service = build_service(&db, &root).await;

    let mut restore_apply = restore_args();
    restore_apply.item_ids = vec![item_id];
    let report = context69::services::library::artifact_backfill::run_restore_apply(
        &db,
        &service,
        &restore_apply,
    )
    .await
    .expect("restore reports a stopped batch");
    assert!(report.error.is_some(), "corrupt bytes must stop the batch");
    assert_eq!(report.restored.download, 0);
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(artifact_count(&db, item_id).await, 1);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn restore_skips_an_item_that_already_has_inline_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let content = b"hello";
    let (object_id, sha256) = seed_verified_object(&db, &root, owner.group_id, content).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": sha256,
            "content_base64": "aGVsbG8="
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    seed_artifact(&db, item_id, KIND_DOWNLOAD, object_id).await;
    let service = build_service(&db, &root).await;

    let preview = context69::services::library::artifact_backfill::run_restore_preview(
        &db,
        &restore_preview_args(),
        "local",
    )
    .await
    .expect("restore preview runs");
    assert_eq!(
        preview.skipped.get("inline_present").copied().unwrap_or(0),
        1
    );

    let mut restore_apply = restore_args();
    restore_apply.item_ids = vec![item_id];
    let report = context69::services::library::artifact_backfill::run_restore_apply(
        &db,
        &service,
        &restore_apply,
    )
    .await
    .expect("restore apply runs");
    assert_eq!(report.restored.download, 0);
    assert_eq!(
        report.skipped.get("inline_present").copied().unwrap_or(0),
        1
    );
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(artifact_count(&db, item_id).await, 1);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn inverse_cas_refuses_changed_identity_without_mutation() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let content = b"hello";
    let (object_id, sha256) = seed_verified_object(&db, &root, owner.group_id, content).await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": sha256.clone()
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    seed_artifact(&db, item_id, KIND_DOWNLOAD, object_id).await;

    let refused = db
        .restore_task_item_artifact_inline(context69::db::RestoreTaskItemArtifactInlineRequest {
            item_id,
            kind: TaskItemArtifactKind::Download,
            storage_object_id: object_id,
            sha256: &"0".repeat(64),
            size_bytes: content.len() as i64,
            storage_backend: "local",
            encoding: ENCODING_DOWNLOAD,
            download_base64: Some("aGVsbG8="),
            sections_json: None,
        })
        .await
        .expect("inverse CAS runs");
    assert!(!refused, "a changed registry identity must not restore");
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert_eq!(artifact_count(&db, item_id).await, 1);
    assert_eq!(
        sqlx::query_file!("tests/task_item_artifacts/open_intents.sql", object_id)
            .fetch_one(db.pool())
            .await
            .expect("open intent query")
            .count
            .unwrap_or(0),
        0,
        "a refused restore must not enqueue a cleanup intent"
    );

    let restored = db
        .restore_task_item_artifact_inline(context69::db::RestoreTaskItemArtifactInlineRequest {
            item_id,
            kind: TaskItemArtifactKind::Download,
            storage_object_id: object_id,
            sha256: &sha256,
            size_bytes: content.len() as i64,
            storage_backend: "local",
            encoding: ENCODING_DOWNLOAD,
            download_base64: Some("aGVsbG8="),
            sections_json: None,
        })
        .await
        .expect("inverse CAS runs");
    assert!(restored, "the exact identity restores the field");
    assert_eq!(artifact_count(&db, item_id).await, 0);
    assert_eq!(
        sqlx::query_file!("tests/task_item_artifacts/open_intents.sql", object_id)
            .fetch_one(db.pool())
            .await
            .expect("open intent query")
            .count
            .unwrap_or(0),
        1,
        "the reference removal enqueues exactly one cleanup intent"
    );
    assert_eq!(
        item_payload(&db, item_id).await["download_artifact"]["content_base64"],
        "aGVsbG8="
    );
    assert_eq!(
        item_payload(&db, item_id).await["download_artifact"]["sha256"],
        sha256
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn restore_keeps_an_object_that_a_library_file_still_owns() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let content = b"shared bytes";
    let (object_id, sha256) = seed_verified_object(&db, &root, owner.group_id, content).await;
    seed_file_reference(
        &db,
        owner.group_id,
        object_id,
        &format!("objects/{}/{sha256}", owner.group_id),
        &sha256,
    )
    .await;
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": sha256
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;
    seed_artifact(&db, item_id, KIND_DOWNLOAD, object_id).await;
    let service = build_service(&db, &root).await;

    let mut restore_apply = restore_args();
    restore_apply.item_ids = vec![item_id];
    let report = context69::services::library::artifact_backfill::run_restore_apply(
        &db,
        &service,
        &restore_apply,
    )
    .await
    .expect("restore apply runs");
    assert!(report.error.is_none());
    assert_eq!(report.restored.download, 1);
    assert_eq!(artifact_count(&db, item_id).await, 0);
    assert_eq!(
        sqlx::query_file!("tests/task_item_artifacts/object_exists.sql", object_id)
            .fetch_one(db.pool())
            .await
            .expect("object row query")
            .count,
        Some(1),
        "a shared library-file reference must keep the object"
    );
    assert!(
        sqlx::query_file!("tests/task_item_artifacts/open_intents.sql", object_id)
            .fetch_one(db.pool())
            .await
            .expect("open intent query")
            .count
            .unwrap_or(0)
            >= 1,
        "the reference removal enqueues the old-object cleanup"
    );
    assert!(std::fs::read(root.join(object_key(&db, object_id).await)).is_ok());

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

/// Issue #975 P8: a deduplicated stage resets the shared staging lease, so an
/// artifact owner can reach the staged release path. Its bytes must survive both
/// the lock guard and the record delete.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn shared_artifact_bytes_survive_a_staging_discard() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let service = build_service(&db, &root).await;
    let content = b"shared artifact bytes";

    let artifact_lease = Uuid::new_v4();
    let (_artifact_task, artifact_item) =
        seed_running_item(&db, &owner, serde_json::json!({}), artifact_lease).await;
    let (object_id, sha) = seed_verified_object(&db, &root, owner.group_id, content).await;
    let key = object_key(&db, object_id).await;
    seed_artifact(&db, artifact_item, KIND_SECTIONS, object_id).await;

    let staging_token = Uuid::new_v4();
    let (_staging_task, staging_item) =
        seed_running_item(&db, &owner, serde_json::json!({}), staging_token).await;
    // The second stage reuses the same content-addressed row and resets its
    // shared 24-hour staging lease.
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(content), staging_token)
        .await
        .expect("stage identical bytes for the second item");
    assert_eq!(staged.object_id, object_id, "the object row is reused");
    assert!(
        staging_lease(&db, object_id).await.is_some(),
        "the staged object holds a lease"
    );

    // The staged item loses its lease before the attach, so the attach refuses
    // and the discard runs the reference-checked staging rollback.
    let lost = service
        .attach_task_item_artifact(
            staging_item,
            Uuid::new_v4(),
            TaskItemArtifactKind::Sections,
            &staged,
        )
        .await
        .expect("a refused attach is not an error");
    assert!(!lost, "a stale lease must not attach");

    assert_eq!(
        artifact_count(&db, artifact_item).await,
        1,
        "the other item's reference must survive"
    );
    assert_eq!(
        object_rows(&db, object_id).await,
        1,
        "the registry row survives"
    );
    assert!(
        staging_lease(&db, object_id).await.is_some(),
        "the shared staging lease is untouched"
    );
    assert!(
        std::fs::read(root.join(&key)).expect("read shared artifact bytes") == content,
        "physical bytes must not be deleted"
    );
    assert_eq!(artifact_count(&db, staging_item).await, 0);
    let _ = sha;

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

/// Issue #975 P8: enumeration excludes artifact owners, and an artifact that
/// appears after enumeration must be refused again by the under-lock re-read
/// before any physical delete.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn orphan_under_lock_guard_refuses_an_artifact_attached_after_enumeration() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_task, item) = seed_task_item(&db, &owner).await;
    let (object_id, key, _sha) = seed_object(&db, &root, owner.group_id, b"orphan bytes").await;
    let expired = (Utc::now() - ChronoDuration::hours(1)).trunc_subsecs(6);
    set_staging_lease(&db, object_id, Some(expired)).await;

    let store = LibraryStore::new(db.clone());
    let enumerated = store
        .sweep_orphaned_storage_objects(Utc::now(), 10)
        .await
        .expect("enumerate orphans");
    assert!(
        enumerated.iter().any(|object| object.id == object_id),
        "an unreferenced staged object is an enumeration candidate"
    );

    // A reference commits between enumeration and the under-lock re-read.
    seed_artifact(&db, item, KIND_DOWNLOAD, object_id).await;

    let mut tx = db.pool().begin().await.expect("begin lock transaction");
    let under_lock = store
        .get_storage_object_by_id_for_update(&mut tx, object_id, Utc::now())
        .await
        .expect("under-lock re-read");
    assert!(
        under_lock.is_none(),
        "an artifact reference must refuse the orphan under the lock"
    );
    assert!(
        !store
            .delete_orphaned_storage_object_record_for_update(&mut tx, object_id, Utc::now())
            .await
            .expect("under-lock record delete"),
        "the record delete must refuse the same artifact reference"
    );
    tx.rollback().await.expect("release lock transaction");

    assert_eq!(object_rows(&db, object_id).await, 1);
    assert_eq!(artifact_count(&db, item).await, 1);
    assert!(
        std::fs::read(root.join(&key)).expect("read orphan bytes") == b"orphan bytes",
        "physical bytes must remain"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

/// Issue #975 P8: a retry can rewrite a previously small field between the
/// metadata scan and the capped fetch. The fetch statement must withhold the
/// text and report the size refusal instead of materializing it.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn capped_field_fetch_refuses_growth_between_scan_and_fetch() {
    let (_guard, db) = prepare().await;
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let small = base64::engine::general_purpose::STANDARD.encode(b"small");
    let payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(b"small"),
            "content_base64": small
        }
    });
    make_terminal(&db, task_id, item_id, payload.clone()).await;

    let scanned = db
        .list_terminal_artifact_states("download", None, 10, Some(&[item_id]))
        .await
        .expect("scan candidates");
    assert_eq!(scanned.len(), 1, "the small field is a candidate");

    let cap = 1024_i64;
    let grown = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 4096]);
    let grown_payload = serde_json::json!({
        "download_artifact": {
            "source_url": "https://example.test/x",
            "filename": "x.bin",
            "media_type": "application/octet-stream",
            "sha256": inline_sha256(&[b'x'; 4096]),
            "content_base64": grown
        }
    });
    make_terminal(&db, task_id, item_id, grown_payload.clone()).await;

    let field = db
        .fetch_artifact_backfill_field(item_id, TaskItemArtifactKind::Download, cap)
        .await
        .expect("capped fetch runs")
        .expect("the item still exists");
    assert_eq!(
        field.skip_reason.as_deref(),
        Some("oversize"),
        "the fetch must classify the grown field"
    );
    assert!(
        field.field_text.is_none(),
        "an oversized field is never projected to the client"
    );
    assert_eq!(
        item_payload(&db, item_id).await,
        grown_payload,
        "the fetch itself must not mutate the payload"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
}

/// Issue #975 P8: remaining-reference totals count every artifact row,
/// including those under a soft-deleted parent, and label the scope.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn remaining_reference_total_includes_soft_deleted_parents() {
    let (_guard, db) = prepare().await;
    let owner = seed_owner(&db).await;
    let (task_id, item_id) = seed_task_item(&db, &owner).await;
    let object_id = Uuid::new_v4();
    let sha = hex_digest();
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_storage_object.sql",
        object_id,
        owner.group_id,
        sha.clone(),
        4_i64,
        format!("objects/{}/{sha}", owner.group_id)
    )
    .execute(db.pool())
    .await
    .expect("seed storage object");
    seed_artifact(&db, item_id, KIND_DOWNLOAD, object_id).await;
    let before = db
        .remaining_artifact_references()
        .await
        .expect("count references");
    assert!(before.total >= 1);

    sqlx::query_file!("tests/task_item_artifacts/soft_delete_task.sql", task_id)
        .execute(db.pool())
        .await
        .expect("soft delete the parent task");

    let after = db
        .remaining_artifact_references()
        .await
        .expect("count references");
    assert_eq!(
        after.total, before.total,
        "a soft-deleted parent still owns its artifact rows"
    );
    assert_eq!(after.soft_deleted, before.soft_deleted + 1);

    cleanup(&db, &[owner.group_id], owner.user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_reference_committed_during_a_discard_lock_wait_keeps_the_checkpoint_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) =
        seed_running_item(&db, &owner, download_payload(b"waited discard"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"waited discard"), lease)
        .await
        .expect("stage artifact");
    let key = object_key(&db, staged.object_id).await;
    assert!(root.join(&key).exists());

    // The attach's INSERT holds FK KEY SHARE on the object row, uncommitted.
    let mut attach = db.pool().begin().await.expect("begin the attach");
    let blocker_pid = backend_pid(&mut attach).await;
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_artifact.sql",
        item,
        KIND_DOWNLOAD,
        staged.object_id,
        ENCODING_DOWNLOAD
    )
    .execute(&mut *attach)
    .await
    .expect("insert the uncommitted reference");
    let discard = tokio::spawn({
        let service = service.clone();
        let staged = staged.clone();
        async move {
            service.discard_maintenance_task_artifact(&staged).await;
        }
    });
    assert!(
        wait_for_object_lock_waiter(&db, blocker_pid).await,
        "the real discard must be observed waiting for the object row lock"
    );
    assert!(
        !discard.is_finished(),
        "the discard must not reach the physical delete before the reference commits"
    );
    attach
        .commit()
        .await
        .expect("commit the reference while the discard waits");
    tokio::time::timeout(std::time::Duration::from_secs(30), discard)
        .await
        .expect("the discard must resume after the commit")
        .expect("join the discard");

    assert_eq!(artifact_count(&db, item).await, 1);
    assert_eq!(object_rows(&db, staged.object_id).await, 1);
    assert!(
        root.join(&key).exists(),
        "the fresh snapshot must see the committed reference and keep the bytes"
    );
    assert!(
        staging_lease(&db, staged.object_id).await.is_some(),
        "the reference-checked rollback leaves the staged record intact"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_reference_committed_during_a_sweep_lock_wait_keeps_the_checkpoint_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (_, item) = seed_task_item(&db, &owner).await;
    let (object, key, _sha) = seed_object(&db, &root, owner.group_id, b"waited sweep").await;
    age_staged_object(&db, object, Utc::now() - ChronoDuration::hours(25)).await;

    let mut attach = db.pool().begin().await.expect("begin the attach");
    let blocker_pid = backend_pid(&mut attach).await;
    sqlx::query_file!(
        "tests/task_item_artifacts/seed_artifact.sql",
        item,
        KIND_SECTIONS,
        object,
        ENCODING_SECTIONS
    )
    .execute(&mut *attach)
    .await
    .expect("insert the uncommitted reference");

    let service = build_service(&db, &root).await;
    let dispatcher =
        SourceCleanupDispatcher::with_fallback_interval(std::time::Duration::from_secs(60));
    let shutdown = CancellationToken::new();
    let worker = tokio::spawn({
        let dispatcher = dispatcher.clone();
        let service = service.clone();
        let shutdown = shutdown.clone();
        async move {
            dispatcher.run(service, shutdown).await;
        }
    });
    assert!(
        wait_for_object_lock_waiter(&db, blocker_pid).await,
        "the real staged sweep must be observed waiting for the object row lock"
    );
    attach
        .commit()
        .await
        .expect("commit the reference while the sweep waits");
    shutdown.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(30), worker)
        .await
        .expect("the sweep must finish its drained pass")
        .expect("join the sweep");

    assert_eq!(artifact_count(&db, item).await, 1);
    assert_eq!(object_rows(&db, object).await, 1);
    assert!(
        root.join(&key).exists(),
        "the fresh snapshot must see the committed reference and keep the bytes"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_reference_free_discard_still_releases_the_staged_bytes() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let lease = Uuid::new_v4();
    let (_, item) = seed_running_item(&db, &owner, download_payload(b"released"), lease).await;
    let service = build_service(&db, &root).await;
    let staged = service
        .stage_task_item_artifact(owner.group_id, Bytes::from_static(b"released"), lease)
        .await
        .expect("stage artifact");
    let key = object_key(&db, staged.object_id).await;

    service.discard_maintenance_task_artifact(&staged).await;

    assert_eq!(artifact_count(&db, item).await, 0);
    assert_eq!(object_rows(&db, staged.object_id).await, 0);
    assert!(
        !root.join(&key).exists(),
        "an unreferenced staging rollback still removes the physical bytes"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_reference_free_expired_object_is_still_reclaimed_by_the_sweep() {
    let (_guard, db) = prepare().await;
    let root = storage_root();
    let owner = seed_owner(&db).await;
    let (object, key, _sha) = seed_object(&db, &root, owner.group_id, b"released sweep").await;
    age_staged_object(&db, object, Utc::now() - ChronoDuration::hours(25)).await;

    let service = build_service(&db, &root).await;
    let dispatcher =
        SourceCleanupDispatcher::with_fallback_interval(std::time::Duration::from_secs(60));
    let shutdown = CancellationToken::new();
    let worker = tokio::spawn({
        let dispatcher = dispatcher.clone();
        let service = service.clone();
        let shutdown = shutdown.clone();
        async move {
            dispatcher.run(service, shutdown).await;
        }
    });
    assert!(
        wait_until_object_is_gone(&db, object).await,
        "the real sweep must reclaim the unreferenced expired object"
    );
    shutdown.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(30), worker)
        .await
        .expect("the sweep must finish its drained pass")
        .expect("join the sweep");

    assert!(
        !root.join(&key).exists(),
        "the sweep still deletes the bytes of a reference-free expired object"
    );

    cleanup(&db, &[owner.group_id], owner.user_id).await;
    let _ = std::fs::remove_dir_all(root);
}
