//! Terminal retirement of unresolved library ingest runs and in-place retry
//! generations (issue 1138 P18).
//!
//! Every case runs against the configured reusable PostgreSQL and drives a
//! terminal producer through its existing database API, so the trigger is
//! exercised in the same transaction that the production paths use. No vector
//! service is touched.

use chrono::Utc;
use context69::db::{
    CreateTaskSubmissionRequest, Database, ExecutionContext, FinishTaskItemRequest, IngestRunMode,
    IngestRunSource, NewIngestRun, RegistryQuotaOverride, WaitTaskItemRequest,
};
use context69::library_store::{LibraryStore, NewLibraryFile, TaskWriteFence, begin_task_write};
use context69_search::SearchRegistryEntry;
use uuid::Uuid;

struct RetirementFixture {
    db: Database,
    group: i64,
    user: i64,
    task_id: Uuid,
    item_id: Uuid,
    file_id: Uuid,
    fence: TaskWriteFence,
    execution: ExecutionContext,
    attempt_id: i64,
}

impl RetirementFixture {
    async fn new() -> Self {
        let db = Database::connect(&context69_test_support::accessor::database_url())
            .await
            .unwrap();
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .unwrap();
        let user = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_user.sql",
            format!("retire-{}", Uuid::new_v4()),
            "Retire Test",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        let group = sqlx::query_file_scalar!(
            "tests/sql/task_execution/seed_group.sql",
            format!("retire-{}", Uuid::new_v4()),
            "Retire Test",
            format!("test/retire-{}", Uuid::new_v4())
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        sqlx::query_file!(
            "tests/sql/task_execution/seed_group_membership.sql",
            group,
            user
        )
        .execute(db.pool())
        .await
        .unwrap();

        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group),
                kind: "text_batch",
                group_path: Some("test/retire"),
                source_key: None,
                payloads: &[serde_json::json!({ "external_id": "a" })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("retire-{}", Uuid::new_v4()),
            })
            .await
            .unwrap();
        let item = db
            .claim_items_fast(50)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.id == items[0])
            .unwrap();
        let execution = item.execution_context().unwrap();
        let fence = TaskWriteFence::from_claimed_item(&item).unwrap();
        let store = LibraryStore::new(db.clone());
        let file = NewLibraryFile {
            id: Uuid::new_v4(),
            folder_id: None,
            external_id: None,
            filename: "retire.txt".into(),
            media_type: "text/plain".into(),
            size_bytes: 4,
            sha256: "0".repeat(64),
            storage_rel_path: format!("objects/test/{}", Uuid::new_v4()),
            storage_object_id: None,
            delete_source_after_processing: false,
        };
        store
            .create_file_in_project_fenced(fence, group, &file)
            .await
            .unwrap()
            .unwrap();
        assert!(
            db.set_task_item_file(task_id, item.id, &execution, file.id)
                .await
                .unwrap()
        );

        Self {
            db,
            group,
            user,
            task_id,
            item_id: item.id,
            file_id: file.id,
            fence,
            execution,
            attempt_id: item.attempt_id,
        }
    }

    fn run_plan() -> NewIngestRun {
        NewIngestRun {
            mode: IngestRunMode::FirstInsert,
            source: IngestRunSource::Stored,
        }
    }

    async fn create_run(&self) -> context69::db::LibraryIngestRun {
        let mut tx = begin_task_write(&self.db, self.fence).await.unwrap();
        let run = self
            .db
            .claim_or_create_library_ingest_run_in(
                &mut tx,
                self.task_id,
                self.file_id,
                Some(Self::run_plan()),
            )
            .await
            .unwrap();
        assert!(tx.commit().await.unwrap());
        run
    }

    async fn admit(&self, run: &context69::db::LibraryIngestRun, staged_ids: &[Uuid]) {
        let entry = SearchRegistryEntry {
            run_id: run.batch_id,
            file_id: Some(self.file_id),
            document_ids: Vec::new(),
            staged_ids: staged_ids.to_vec(),
            retired_ids: Vec::new(),
            baseline_record_hash: None,
            staged_record_hash: None,
            generation: run.generation,
            phase: "staging".into(),
        };
        let mut tx = self.db.pool().begin().await.unwrap();
        let epoch = self
            .db
            .admit_search_registry_entry_in(
                &entry,
                2,
                RegistryQuotaOverride::exact([1000, 1000, 1000]),
                &mut tx,
            )
            .await
            .unwrap();
        assert!(epoch.is_some(), "the staging admission must be accepted");
        tx.commit().await.unwrap();
    }

    async fn finish(&self, status: &str) -> bool {
        self.db
            .finish_task_item(FinishTaskItemRequest {
                task_id: self.task_id,
                item_id: self.item_id,
                status,
                resource_id: None,
                failure_stage: Some("replacement"),
                error_message: Some("terminal"),
                retryable: false,
                execution: self.execution.clone(),
                attempt_id: self.attempt_id,
            })
            .await
            .unwrap()
    }

    async fn entry_phase(&self, run_id: Uuid) -> (String, Vec<Uuid>, Vec<Uuid>) {
        let state = self.db.read_search_registry_state().await.unwrap();
        let entry = state
            .entries
            .into_iter()
            .find(|entry| entry.run_id == run_id)
            .expect("the registry entry exists");
        (entry.phase, entry.retired_ids, entry.staged_ids)
    }

    async fn item_status(&self) -> String {
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(self.item_id)
            .fetch_one(self.db.pool())
            .await
            .unwrap()
    }

    async fn cleanup(&self) {
        let history = self
            .db
            .read_library_ingest_run_history(self.item_id)
            .await
            .unwrap();
        for run in history {
            sqlx::query_file!(
                "tests/library_ingest_run_retirement/delete_registry_entry.sql",
                run.batch_id
            )
            .execute(self.db.pool())
            .await
            .unwrap();
        }
        sqlx::query_file!(
            "tests/sql/task_execution/delete_task_by_id.sql",
            self.task_id
        )
        .execute(self.db.pool())
        .await
        .unwrap();
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_files.sql",
            self.group
        )
        .execute(self.db.pool())
        .await
        .unwrap();
        sqlx::query_file!("tests/sql/task_execution/delete_group.sql", self.group)
            .execute(self.db.pool())
            .await
            .unwrap();
        sqlx::query_file!("tests/sql/task_execution/delete_user.sql", self.user)
            .execute(self.db.pool())
            .await
            .unwrap();
    }
}

/// A failed finish retires the unresolved run in the same transaction and
/// moves its staging entry to its tombstone form with exactly one epoch bump.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_finish_retires_the_run_and_tombstones_the_staging_entry() {
    let fixture = RetirementFixture::new().await;
    let run = fixture.create_run().await;
    let staged = vec![Uuid::new_v4(), Uuid::new_v4()];
    fixture.admit(&run, &staged).await;
    let epoch_before = fixture.db.read_search_registry_epoch().await.unwrap();

    assert!(fixture.finish("failed").await);
    assert_eq!(fixture.item_status().await, "failed");

    let current = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap();
    assert!(current.is_none(), "a retired run is never current");
    let history = fixture
        .db
        .read_library_ingest_run_history(fixture.item_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "the retired row is preserved as history");
    assert_eq!(history[0].batch_id, run.batch_id);
    assert_eq!(history[0].generation, run.generation);
    assert!(history[0].retired_at.is_some());
    assert!(history[0].resolved_at.is_none());

    let (phase, retired, staged_after) = fixture.entry_phase(run.batch_id).await;
    assert_eq!(phase, "retired");
    assert_eq!(staged_after, Vec::<Uuid>::new());
    assert_eq!(retired, staged, "the full staged set is tombstoned");
    assert_eq!(
        fixture.db.read_search_registry_epoch().await.unwrap(),
        epoch_before + 1,
        "retirement bumps the epoch exactly once"
    );

    fixture.cleanup().await;
}

/// A terminal cancellation with no admitted staging entry retires the run
/// without touching the registry or its epoch.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_task_retires_without_registry_mutation() {
    let fixture = RetirementFixture::new().await;
    let run = fixture.create_run().await;
    let epoch_before = fixture.db.read_search_registry_epoch().await.unwrap();

    assert!(
        fixture
            .db
            .cancel_task(fixture.task_id, fixture.user)
            .await
            .unwrap()
    );
    assert_eq!(fixture.item_status().await, "cancelled");

    let history = fixture
        .db
        .read_library_ingest_run_history(fixture.item_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].retired_at.is_some());
    assert_eq!(history[0].batch_id, run.batch_id);
    assert_eq!(
        fixture.db.read_search_registry_epoch().await.unwrap(),
        epoch_before,
        "an unadmitted run retires without a registry mutation"
    );

    fixture.cleanup().await;
}

/// A retryable wait keeps the same current run; exhausted maintenance later
/// fails the item and retires it.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_wait_reuses_the_current_run_and_exhaustion_retires_it() {
    let fixture = RetirementFixture::new().await;
    let run = fixture.create_run().await;
    let epoch_before = fixture.db.read_search_registry_epoch().await.unwrap();

    assert!(
        fixture
            .db
            .wait_task_item(WaitTaskItemRequest {
                task_id: fixture.task_id,
                item_id: fixture.item_id,
                execution: fixture.execution.clone(),
                waiting_reason: "backoff",
                dependency_key: None,
                next_attempt_at: Utc::now(),
                error_message: Some("transient"),
            })
            .await
            .unwrap()
    );
    let current = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap()
        .expect("a wait must not retire the run");
    assert_eq!(current.batch_id, run.batch_id);
    assert_eq!(current.generation, run.generation);
    assert_eq!(
        fixture.db.read_search_registry_epoch().await.unwrap(),
        epoch_before
    );

    sqlx::query_file!(
        "tests/library_ingest_run_retirement/exhaust_item.sql",
        fixture.item_id
    )
    .execute(fixture.db.pool())
    .await
    .unwrap();
    fixture.db.maintain_claim_state().await.unwrap();
    assert_eq!(fixture.item_status().await, "failed");

    let history = fixture
        .db
        .read_library_ingest_run_history(fixture.item_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].retired_at.is_some());

    fixture.cleanup().await;
}

/// A run that resolved through a committed switch is a success and stays
/// current even when the item later terminates.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_resolved_switched_run_is_never_retired() {
    let fixture = RetirementFixture::new().await;
    let run = fixture.create_run().await;
    fixture.admit(&run, &[Uuid::new_v4()]).await;
    let old_live = vec![Uuid::new_v4()];

    let mut connection = fixture.db.pool().acquire().await.unwrap();
    assert!(
        fixture
            .db
            .transition_search_registry_entry_in(
                run.batch_id,
                "staging",
                "switched",
                Some(&old_live),
                &mut connection,
            )
            .await
            .unwrap()
            .is_some()
    );
    drop(connection);
    let mut tx = begin_task_write(&fixture.db, fixture.fence).await.unwrap();
    assert!(
        fixture
            .db
            .mark_library_ingest_run_resolved_in(&mut tx, fixture.task_id, fixture.file_id)
            .await
            .unwrap()
    );
    assert!(tx.commit().await.unwrap());
    let epoch_before = fixture.db.read_search_registry_epoch().await.unwrap();

    assert!(fixture.finish("failed").await);

    let current = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap()
        .expect("a resolved successful run is never retired");
    assert_eq!(current.batch_id, run.batch_id);
    assert!(current.retired_at.is_none());
    assert!(current.resolved_at.is_some());
    let (phase, retired, _) = fixture.entry_phase(run.batch_id).await;
    assert_eq!(phase, "switched");
    assert_eq!(retired, old_live);
    assert_eq!(
        fixture.db.read_search_registry_epoch().await.unwrap(),
        epoch_before
    );

    fixture.cleanup().await;
}

/// A same-item retry after retirement allocates a fresh current generation,
/// keeps the retired history readable, and cannot insert a second current row.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_retry_allocates_a_fresh_generation_and_keeps_history() {
    let fixture = RetirementFixture::new().await;
    let run = fixture.create_run().await;
    fixture.admit(&run, &[Uuid::new_v4()]).await;
    assert!(fixture.finish("failed").await);

    let retried = fixture
        .db
        .retry_task_items(fixture.task_id, fixture.user)
        .await
        .unwrap();
    assert_eq!(retried, vec![fixture.item_id]);
    let next = fixture
        .db
        .claim_items_fast(50)
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == fixture.item_id)
        .unwrap();
    let fence = TaskWriteFence::from_claimed_item(&next).unwrap();
    let mut tx = begin_task_write(&fixture.db, fence).await.unwrap();
    let fresh = fixture
        .db
        .claim_or_create_library_ingest_run_in(
            &mut tx,
            fixture.task_id,
            fixture.file_id,
            Some(RetirementFixture::run_plan()),
        )
        .await
        .unwrap();
    assert!(tx.commit().await.unwrap());

    assert_ne!(fresh.batch_id, run.batch_id);
    assert!(fresh.generation > run.generation);
    assert!(fresh.retired_at.is_none());
    let history = fixture
        .db
        .read_library_ingest_run_history(fixture.item_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].batch_id, run.batch_id);
    assert!(history[0].retired_at.is_some());
    assert_eq!(history[1].batch_id, fresh.batch_id);
    assert!(history[1].retired_at.is_none());
    let current = fixture
        .db
        .read_library_ingest_run(fixture.item_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.batch_id, fresh.batch_id);

    let duplicate = sqlx::query_file!(
        "tests/library_ingest_run_retirement/insert_current_run.sql",
        fixture.item_id,
        fixture.task_id,
        fixture.file_id,
        Uuid::new_v4(),
        "first_insert",
        "stored",
        fresh.lease_token,
        fresh.execution_task_id,
        fresh.execution_generation
    )
    .execute(fixture.db.pool())
    .await;
    assert!(
        duplicate.is_err(),
        "the partial unique index refuses a second current run"
    );

    fixture.cleanup().await;
}

/// Two concurrent terminal finishes retire the run exactly once.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_terminal_finishes_retire_the_run_once() {
    let fixture = RetirementFixture::new().await;
    let run = fixture.create_run().await;
    fixture.admit(&run, &[Uuid::new_v4()]).await;
    let epoch_before = fixture.db.read_search_registry_epoch().await.unwrap();

    let first = fixture.finish("failed");
    let second = fixture.finish("failed");
    let (first, second) = tokio::join!(first, second);
    assert_ne!(
        first, second,
        "exactly one concurrent terminal finish owns the transition"
    );

    let history = fixture
        .db
        .read_library_ingest_run_history(fixture.item_id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].retired_at.is_some());
    assert_eq!(
        fixture.db.read_search_registry_epoch().await.unwrap(),
        epoch_before + 1,
        "the retirement, not the losing writer, bumps the epoch"
    );

    fixture.cleanup().await;
}
