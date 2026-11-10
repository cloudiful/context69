//! The bounded batch source rerun and the public succeeded rerun (issue 1138
//! P28).
//!
//! Only a succeeded file, url, or text parent is rerun from its current
//! source: the child reads the source again instead of reusing the parent's
//! parsed result. Every other id is reported with a reason instead of being
//! forked, and the outcomes keep the requested order. These cases run only
//! when CONTEXT69_TEST_DATABASE_URL points to a scratch database and are
//! skipped otherwise.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::domain::GroupRecord;
use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
use context69::library_store::{LibraryStore, NewLibraryFile};
use context69::services::document_store::DocumentStoreService;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::namespace::NamespaceService;
use context69::services::secret_store;
use context69::services::settings::SettingsService;
use context69::services::source_folders::SourceFoldersService;
use context69::services::sync::SyncService;
use context69::services::tasks::{TaskService, TaskServiceDependencies};
use context69::services::translation::TranslationPublisherAdapter;
use context69_contracts::{TaskBatchRerunOutcome, TaskBatchRerunReason, TaskBatchRerunRequest};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

/// The claim path is global, so these cases serialize among themselves.
static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct FakeEmbedding;

#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32; 4])
            .collect())
    }
}

struct ReadyTranslation;

#[async_trait]
impl TranslationReadiness for ReadyTranslation {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

async fn connect_db() -> Database {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("batch-rerun-{}", Uuid::new_v4()))
    .bind("Batch Rerun")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

async fn seed_group(db: &Database, owner_id: i64) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, full_path, name, visibility, kind) \
         VALUES ($1, $2, $3, 'public', 'shared') \
         RETURNING id, group_key, full_path, name, created_at, updated_at",
    )
    .bind(format!("batch-rerun-{}", Uuid::new_v4()))
    .bind(format!("test/batch-rerun-{}", Uuid::new_v4()))
    .bind("Batch Rerun")
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    // The owner membership is what makes the group's tasks manageable.
    sqlx::query(
        "INSERT INTO context69.group_memberships (group_id, user_id, role) \
         VALUES ($1, $2, 'owner') ON CONFLICT DO NOTHING",
    )
    .bind(row.get::<i64, _>("id"))
    .bind(owner_id)
    .execute(db.pool())
    .await
    .expect("seed group membership");
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

/// A library file the source rerun can read again.
async fn seed_file(
    db: &Database,
    group: &GroupRecord,
    external_id: Option<&str>,
) -> (Uuid, PathBuf) {
    seed_named_file(db, group, external_id, "source.txt").await
}

/// A library file with its own name, so one group can hold several sources.
async fn seed_named_file(
    db: &Database,
    group: &GroupRecord,
    external_id: Option<&str>,
    filename: &str,
) -> (Uuid, PathBuf) {
    let storage_root = std::env::temp_dir().join(format!("batch-rerun-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let body = b"batch rerun source body\n".to_vec();
    let digest = Sha256::digest(&body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let storage_rel_path = format!("objects/{}.txt", Uuid::new_v4());
    let path = storage_root.join(&storage_rel_path);
    std::fs::create_dir_all(path.parent().expect("object parent")).expect("create object dir");
    std::fs::write(&path, &body).expect("write source bytes");
    let store = LibraryStore::new(db.clone());
    let file = store
        .create_file_in_project(
            group.id,
            &NewLibraryFile {
                id: Uuid::new_v4(),
                folder_id: None,
                external_id: external_id.map(str::to_owned),
                filename: filename.into(),
                media_type: "text/plain".into(),
                size_bytes: body.len() as i64,
                sha256: digest,
                storage_rel_path: storage_rel_path.clone(),
                storage_object_id: None,
                delete_source_after_processing: false,
            },
        )
        .await
        .expect("seed library file");
    (file.id, storage_root)
}

/// A task whose item already succeeded against `file_id`.
async fn seed_task(
    db: &Database,
    user_id: i64,
    group: &GroupRecord,
    kind: &str,
    file_id: Option<Uuid>,
    status: &str,
) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: Some(group.id),
            kind,
            group_path: Some("test/batch-rerun"),
            source_key: None,
            payloads: &[serde_json::json!({
                "filename": "source.txt",
                "media_type": "text/plain",
                "options": { "metadata": {}, "source_policy": "retain" },
            })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("batch-rerun-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    let item_id = item_ids[0];
    if let Some(file_id) = file_id {
        sqlx::query("UPDATE context69.task_items SET file_id = $2 WHERE id = $1")
            .bind(item_id)
            .bind(file_id)
            .execute(db.pool())
            .await
            .expect("attach file");
    }
    sqlx::query(
        "UPDATE context69.task_items SET status = $2, finished_at = now(), \
         lease_token = NULL, lease_until = NULL WHERE id = $1",
    )
    .bind(item_id)
    .bind(if status == "succeeded" {
        "succeeded"
    } else {
        "failed"
    })
    .execute(db.pool())
    .await
    .expect("mark item");
    sqlx::query(
        "UPDATE context69.tasks SET status = $2, succeeded_count = $3, queued_count = 0, \
         failed_count = $4, finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .bind(status)
    .bind(i64::from(status == "succeeded"))
    .bind(i64::from(status == "failed"))
    .execute(db.pool())
    .await
    .expect("mark task");
    (task_id, item_id)
}

async fn build_service(db: &Database, storage_root: &PathBuf) -> TaskService {
    let chunking = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(TranslationPublisherAdapter::new(
            None,
            None,
            chunking.clone(),
        )),
        concurrency: 1,
        readiness: Arc::new(ReadyTranslation),
    });
    let library = LibraryService::new(
        db.clone(),
        EmbeddingRuntime::new(Some(Arc::new(FakeEmbedding)), None),
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root: storage_root.clone(),
                max_upload_size_mb: 10,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: true,
            embedding_vector_configuration_fingerprint: "batch-rerun".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
    )
    .await
    .expect("build library service");
    let sync = SyncService::new(
        db.clone(),
        secret_store::build_unkeyed(db),
        None,
        None,
        chunking,
        1,
        translation.clone(),
    );
    TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
        concurrency: 1,
    })
}

async fn task_origin(db: &Database, task_id: Uuid) -> String {
    sqlx::query_scalar("SELECT origin FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("read task origin")
}

/// The batch reruns a succeeded parent into a fresh child and reports every
/// other id with a reason, in the requested order.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_batch_reruns_only_the_succeeded_parent_and_keeps_the_request_order() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let other_user = seed_user(&db).await;
    let group = seed_group(&db, user).await;
    // A group the caller does not manage: its task is unreadable for them.
    let foreign_group = seed_group(&db, other_user).await;
    let (file_id, storage_root) = seed_file(&db, &group, None).await;
    let (succeeded, _) =
        seed_task(&db, user, &group, "file_batch", Some(file_id), "succeeded").await;
    let (failed, _) = seed_task(&db, user, &group, "file_batch", Some(file_id), "failed").await;
    let (foreign, _) = seed_task(
        &db,
        other_user,
        &foreign_group,
        "file_batch",
        None,
        "succeeded",
    )
    .await;
    let missing = Uuid::new_v4();
    let service = build_service(&db, &storage_root).await;

    let response = service
        .batch_rerun(
            user,
            &TaskBatchRerunRequest {
                task_ids: vec![succeeded, failed, foreign, missing],
            },
        )
        .await
        .expect("batch rerun");

    assert_eq!(response.results.len(), 4, "one outcome per requested id");
    assert_eq!(response.results[0].task_id, succeeded);
    assert_eq!(response.results[0].outcome, TaskBatchRerunOutcome::Accepted);
    assert_eq!(response.results[0].reason, None);
    let rerun_task_id = response.results[0]
        .rerun_task_id
        .expect("an accepted id carries the created child");
    assert_ne!(
        rerun_task_id, succeeded,
        "a source rerun forks a child and never reuses the parent id"
    );
    assert_eq!(task_origin(&db, rerun_task_id).await, "rerun_source");

    assert_eq!(response.results[1].task_id, failed);
    assert_eq!(response.results[1].outcome, TaskBatchRerunOutcome::Skipped);
    assert_eq!(
        response.results[1].reason,
        Some(TaskBatchRerunReason::IneligibleStatus),
        "a failed parent belongs to the in-place recovery action"
    );
    assert_eq!(response.results[1].rerun_task_id, None);

    for index in [2, 3] {
        assert_eq!(
            response.results[index].outcome,
            TaskBatchRerunOutcome::Skipped
        );
        assert_eq!(
            response.results[index].reason,
            Some(TaskBatchRerunReason::NotFound),
            "a foreign and a missing id are indistinguishable"
        );
        assert_eq!(response.results[index].rerun_task_id, None);
    }
    assert_eq!(response.results[2].task_id, foreign);
    assert_eq!(response.results[3].task_id, missing);
}

/// An active sibling already covers the source, so the rerun is refused for
/// that id alone and never as a generic failure.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_active_source_is_skipped_as_busy() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let group = seed_group(&db, user).await;
    let (file_id, storage_root) = seed_file(&db, &group, None).await;
    let (succeeded, _) =
        seed_task(&db, user, &group, "file_batch", Some(file_id), "succeeded").await;
    // A live item on the same file blocks the source rerun.
    let (sibling, sibling_item) =
        seed_task(&db, user, &group, "file_batch", Some(file_id), "failed").await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'queued', finished_at = NULL WHERE id = $1",
    )
    .bind(sibling_item)
    .execute(db.pool())
    .await
    .expect("activate the sibling item");
    let service = build_service(&db, &storage_root).await;

    let response = service
        .batch_rerun(
            user,
            &TaskBatchRerunRequest {
                task_ids: vec![succeeded],
            },
        )
        .await
        .expect("batch rerun");

    assert_eq!(response.results[0].outcome, TaskBatchRerunOutcome::Skipped);
    assert_eq!(
        response.results[0].reason,
        Some(TaskBatchRerunReason::ActiveSourceBusy)
    );
    assert_eq!(response.results[0].rerun_task_id, None);
    assert_eq!(
        task_origin(&db, sibling).await,
        "manual",
        "the blocking sibling is untouched"
    );
}

/// A managed source-folder file is not a rerunnable source, so the id is
/// reported without a child.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_managed_source_folder_file_is_not_rerunnable() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let group = seed_group(&db, user).await;
    let (file_id, storage_root) = seed_file(
        &db,
        &group,
        Some(&format!("source-folder:{}", Uuid::new_v4())),
    )
    .await;
    let (succeeded, _) =
        seed_task(&db, user, &group, "file_batch", Some(file_id), "succeeded").await;
    let service = build_service(&db, &storage_root).await;

    let response = service
        .batch_rerun(
            user,
            &TaskBatchRerunRequest {
                task_ids: vec![succeeded],
            },
        )
        .await
        .expect("batch rerun");

    assert_eq!(response.results[0].outcome, TaskBatchRerunOutcome::Skipped);
    assert_eq!(
        response.results[0].reason,
        Some(TaskBatchRerunReason::NothingToRerun)
    );
    assert_eq!(response.results[0].rerun_task_id, None);
}

/// The public single rerun accepts a succeeded source parent and returns the
/// created child, while the legacy failed fork and cancelled resume keep their
/// own semantics.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_single_rerun_creates_a_child_for_a_succeeded_source() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let group = seed_group(&db, user).await;
    let (file_id, storage_root) = seed_file(&db, &group, None).await;
    // The failed parent owns another file: the source rerun above already
    // holds the processing slot of the first one.
    let (failed_file_id, _) = seed_named_file(&db, &group, None, "failed-source.txt").await;
    let (succeeded, _) =
        seed_task(&db, user, &group, "file_batch", Some(file_id), "succeeded").await;
    let (failed, failed_item) = seed_task(
        &db,
        user,
        &group,
        "file_batch",
        Some(failed_file_id),
        "failed",
    )
    .await;
    let service = build_service(&db, &storage_root).await;

    let rerun = service.rerun(succeeded, user).await.expect("source rerun");
    assert_ne!(
        rerun.task.task_id, succeeded,
        "the source rerun reports the created child"
    );
    assert_eq!(task_origin(&db, rerun.task.task_id).await, "rerun_source");
    assert_eq!(
        rerun.task.item_ids.len(),
        1,
        "the child carries the parent's rerunnable item"
    );
    assert_eq!(
        task_origin(&db, succeeded).await,
        "manual",
        "the source parent is unchanged"
    );

    // The legacy failed rerun still forks a fresh parent, and the failed item
    // is the one that is carried over.
    let legacy = service
        .rerun(failed, user)
        .await
        .expect("legacy failed rerun");
    assert_ne!(legacy.task.task_id, failed);
    assert_eq!(
        task_origin(&db, legacy.task.task_id).await,
        "rerun",
        "a failed parent keeps the documented fresh-parent rerun"
    );
    assert_eq!(legacy.task.item_ids.len(), 1);
    assert_ne!(
        legacy.task.item_ids[0], failed_item,
        "the legacy fork creates its own item"
    );
}

/// A task kind that owns no rerunnable source is refused by the single route.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unsupported_kind_is_refused_by_the_single_rerun() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let group = seed_group(&db, user).await;
    let (file_id, storage_root) = seed_file(&db, &group, None).await;
    let (translation, _) =
        seed_task(&db, user, &group, "translation", Some(file_id), "succeeded").await;
    let service = build_service(&db, &storage_root).await;

    let error = service
        .rerun(translation, user)
        .await
        .expect_err("a translation task has no rerunnable source");
    let message = error.to_string();
    assert!(
        message.contains("file, url, or text"),
        "the refusal names the rerunnable kinds: {message}"
    );
}
