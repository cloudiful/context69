//! Durable global task-queue admission pause/resume (issue 1212 P2).
//!
//! The pause is a deployment-wide admission gate, not a cancellation: a task
//! that already holds its lease keeps it, a claim that starts after the pause
//! commits admits nothing, and a resume admits exactly the queued work without
//! spending an attempt on the paused wait. The state lives in a row, so a second
//! database handle — the shape a restart takes — observes it.
//!
//! The queue-control singleton is global, so this scenario is a dedicated,
//! serialized test target: it restores the unpaused state on entry and on exit.
//! It runs only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch database.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::domain::UserRecord;
use context69::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    services::{
        document_store::DocumentStoreService,
        library::{LibraryService, LibraryServiceConfig},
        namespace::NamespaceService,
        secret_store,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{TaskService, TaskServiceDependencies},
        translation::TranslationPublisherAdapter,
    },
};
use context69_contracts::{TaskQueueStateResponse, UpdateTaskQueueStateRequest};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use serde_json::json;
use uuid::Uuid;

/// Readiness only: this scenario never runs translation.
struct NotReady;

#[async_trait]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// The production service graph with no network runtimes, so the admin control
/// methods run their real authorization and database paths.
async fn build_task_service(db: &Database) -> TaskService {
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
        readiness: Arc::new(NotReady),
    });
    let storage_root =
        std::env::temp_dir().join(format!("context69-queue-pause-{}", Uuid::new_v4()));
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root,
                max_upload_size_mb: 1,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "queue-pause-test".to_string(),
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

fn actor(is_admin: bool) -> UserRecord {
    let now = Utc::now();
    UserRecord {
        id: 1,
        login_name: "queue-pause-test".to_string(),
        display_name: "Queue Pause Test".to_string(),
        password_hash: "unused".to_string(),
        is_admin,
        disabled_at: None,
        created_at: now,
        updated_at: now,
    }
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("queue-pause-{}", Uuid::new_v4()),
        "Queue Pause Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
}

/// One task with one queued item, at the initial stage.
async fn submit_one_item_task(db: &Database, user_id: i64, label: &str) -> Uuid {
    let task_id = Uuid::new_v4();
    let (task_id, reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/queue-pause"),
            source_key: None,
            payloads: &[json!({"external_id": label})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "queue-pause-hash",
        })
        .await
        .expect("create task");
    assert!(!reused, "a fresh submission creates a new task");
    assert_eq!(item_ids.len(), 1, "the submission carries one item");
    task_id
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_durable_pause_stops_new_claims_and_a_resume_admits_the_queued_work() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");

    // An interrupted earlier run must not leave the deployment paused.
    db.set_task_queue_paused(false)
        .await
        .expect("start unpaused");
    assert!(
        !db.read_task_queue_state()
            .await
            .expect("read the initial state")
            .paused,
        "a deployment starts unpaused, so an old installation keeps admitting work"
    );

    let service = build_task_service(&db).await;
    let admin = actor(true);
    let non_admin = actor(false);

    // A non-admin can neither read nor write the deployment-wide state.
    assert!(
        service.admin_task_queue_state(&non_admin).await.is_err(),
        "a non-admin must not read the queue state"
    );
    assert!(
        service
            .admin_update_task_queue_state(
                &non_admin,
                &UpdateTaskQueueStateRequest { paused: true }
            )
            .await
            .is_err(),
        "a non-admin must not change the queue state"
    );
    assert!(
        !db.read_task_queue_state()
            .await
            .expect("read after the denied write")
            .paused,
        "a denied write changes nothing"
    );

    // A task that already holds its lease keeps it through the pause.
    let user_id = seed_test_user(&db).await;
    let running_task = submit_one_item_task(&db, user_id, "running").await;
    let running = db
        .claim_items(10)
        .await
        .expect("claim the first item")
        .into_iter()
        .find(|item| item.task_id == running_task)
        .expect("the running item is claimed");
    assert_eq!(running.attempt_count, 1);

    // The pause commits durably.
    let paused: TaskQueueStateResponse = service
        .admin_update_task_queue_state(&admin, &UpdateTaskQueueStateRequest { paused: true })
        .await
        .expect("pause the queue");
    assert!(paused.paused);

    // A second handle — the shape a restart takes — observes the same state.
    let observer = Database::connect(&url)
        .await
        .expect("connect a second handle");
    assert!(
        observer
            .read_task_queue_state()
            .await
            .expect("read the paused state")
            .paused,
        "the pause survives a new database handle"
    );

    // A queued item is not admitted while paused, on either claim entrypoint.
    let queued_task = submit_one_item_task(&db, user_id, "queued").await;
    assert!(
        db.claim_items(10)
            .await
            .expect("normal claim while paused")
            .is_empty(),
        "the normal claim path admits nothing while paused"
    );
    assert!(
        db.claim_items_fast(10)
            .await
            .expect("fast claim while paused")
            .is_empty(),
        "the fast claim path admits nothing while paused"
    );

    // The running item still owns its lease, and the paused wait spent no
    // attempt on the queued one.
    let running_task_row = db
        .get_task_internal(running_task)
        .await
        .expect("read the running task")
        .expect("the running task exists");
    assert_eq!(
        running_task_row.status, "running",
        "a pause is never a cancellation"
    );
    let running_items = db
        .list_task_items(running_task, 10, 0)
        .await
        .expect("list the running task items");
    assert_eq!(running_items.len(), 1);
    assert_eq!(running_items[0].status, "running");
    assert_eq!(
        running_items[0].attempt_count, 1,
        "the paused claims never touched the running item"
    );
    let queued_items = db
        .list_task_items(queued_task, 10, 0)
        .await
        .expect("list the queued task items");
    assert_eq!(queued_items.len(), 1);
    assert_eq!(queued_items[0].status, "queued");
    assert_eq!(
        queued_items[0].attempt_count, 0,
        "waiting behind a pause never spends an attempt"
    );

    // A resume admits exactly the queued item, at its first attempt.
    let resumed = service
        .admin_update_task_queue_state(&admin, &UpdateTaskQueueStateRequest { paused: false })
        .await
        .expect("resume the queue");
    assert!(!resumed.paused);
    let claimed = db.claim_items(10).await.expect("claim after the resume");
    let queued_claim = claimed
        .iter()
        .find(|item| item.task_id == queued_task)
        .expect("the queued item is admitted after the resume");
    assert_eq!(
        queued_claim.attempt_count, 1,
        "the resumed item starts at its first attempt"
    );
    assert!(
        !claimed.iter().any(|item| item.task_id == running_task),
        "the item that kept its lease through the pause is never re-claimed"
    );

    // Restore the shared state so the target cannot leave the queue paused.
    db.set_task_queue_paused(false)
        .await
        .expect("restore unpaused");
}
