//! Regression tests for global task execution admission (issue 1172 P2F).
//!
//! The durable library queue policy is the only execution-capacity owner: the
//! namespace `context69` queue `items` bounds how many attempts run at once
//! across every replica, and the domain parent holds no admission slot. These
//! tests pin that contract at the database boundary (no live dispatcher):
//! concurrent claimers never exceed the policy, a capped claim leaves queued
//! work untouched, and releasing an attempt admits the next queued task.
//!
//! Like the other task suites these run only when
//! `CONTEXT69_TEST_DATABASE_URL` points at a scratch database (migrations are
//! applied automatically) and are skipped otherwise. Admission is a global
//! primitive over that shared scratch database, so the file serializes its own
//! cases.

use cloudiful_tasks::{Failure, QueuePolicy};
use context69::db::{ClaimedItem, CreateTaskSubmissionRequest, Database, FinishTaskItemRequest};
use futures::FutureExt;
use serde_json::json;
use uuid::Uuid;

static ADMISSION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();

    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("parent-admission-{}", Uuid::new_v4()),
        "Parent Admission Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
}

async fn seed_task(
    db: &Database,
    user_id: i64,
    payloads: &[serde_json::Value],
    hash: &str,
) -> (Uuid, Vec<Uuid>) {
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/parent-admission"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: hash,
        })
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), payloads.len());
    (task_id, item_ids)
}

async fn item_status(db: &Database, item_id: Uuid) -> String {
    sqlx::query_file_scalar!("tests/sql/task_execution/read_item_status.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("load item status")
}

async fn item_attempt_count(db: &Database, item_id: Uuid) -> i32 {
    sqlx::query_scalar("SELECT attempt_count FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("load item attempt count")
}

async fn parent_started_at(db: &Database, task_id: Uuid) -> Option<chrono::DateTime<chrono::Utc>> {
    sqlx::query_scalar("SELECT started_at FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("load parent start time")
}

async fn finish(db: &Database, item: context69::db::ClaimedItem) {
    db.finish_task_item(FinishTaskItemRequest {
        task_id: item.task_id,
        item_id: item.id,
        status: "succeeded",
        resource_id: None,
        failure_stage: None,
        error_message: None,
        retryable: true,
        execution: item
            .execution_context()
            .expect("the claimed item carries its captured execution"),
        attempt_id: item.attempt_id,
    })
    .await
    .expect("finish the claimed item");
}

/// Assert every grant a claim returned belongs to this test's known fixtures
/// and return them all, so no sibling grant is silently dropped.
fn own_claims(
    returned: Vec<ClaimedItem>,
    fixture_tasks: &[Uuid],
    fixture_items: &[Uuid],
) -> Vec<ClaimedItem> {
    for item in &returned {
        assert!(
            fixture_tasks.contains(&item.task_id) && fixture_items.contains(&item.id),
            "claim returned a grant outside this test's fixtures: \
             task_id={} item_id={} fixture_tasks={fixture_tasks:?} fixture_items={fixture_items:?}",
            item.task_id,
            item.id
        );
    }
    returned
}

/// Seal the exact live library attempt this test owns before its domain rows
/// are deleted. An already terminal or expired grant is confirmed from scoped
/// library history, never inferred from `LeaseLost` and never renewed.
async fn seal_test_execution(db: &Database, item: &ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let result = queue
        .fail(
            &execution.attempt,
            &Failure::permanent(Some(json!({"reason": "context69_test_cleanup"}))),
        )
        .await;
    match result {
        Ok(_) => {}
        Err(
            error @ (cloudiful_tasks::Error::LeaseLost { .. }
            | cloudiful_tasks::Error::InvalidTaskState { .. }),
        ) => {
            let attempts = queue
                .attempts(execution.attempt.task_id(), 16)
                .await
                .expect("read the captured task attempts");
            let sealed = attempts.iter().any(|attempt| {
                attempt.id() == execution.attempt.id() && attempt.status().is_sealed()
            });
            assert!(
                sealed,
                "test cleanup must seal the captured attempt or confirm it already sealed: {error}"
            );
        }
        Err(error) => panic!("test cleanup could not seal the captured attempt: {error}"),
    }
}

/// Failure-safe teardown for the admission case that owns the global policy.
///
/// It records only this test's tasks, user and grants, seals the grants before
/// deleting their rows, and restores the queue policy observed before the test
/// ran instead of blindly freeing the queue. Release is always awaited on the
/// test task; on a body panic the original failure is propagated afterwards.
struct AdmissionScope<'a> {
    db: &'a Database,
    previous_policy: QueuePolicy,
    claims: Vec<ClaimedItem>,
    tasks: Vec<Uuid>,
    users: Vec<i64>,
}

impl<'a> AdmissionScope<'a> {
    async fn new(db: &'a Database) -> Self {
        let previous_policy = db
            .execution_queue()
            .await
            .expect("load the execution queue")
            .policy()
            .await
            .expect("read the queue policy");
        Self {
            db,
            previous_policy,
            claims: Vec::new(),
            tasks: Vec::new(),
            users: Vec::new(),
        }
    }

    fn own_claims(
        &mut self,
        returned: Vec<ClaimedItem>,
        fixture_tasks: &[Uuid],
        fixture_items: &[Uuid],
    ) -> Vec<ClaimedItem> {
        let returned = own_claims(returned, fixture_tasks, fixture_items);
        self.claims.extend(returned.iter().cloned());
        returned
    }

    fn track_task(&mut self, task_id: Uuid) {
        self.tasks.push(task_id);
    }

    fn track_user(&mut self, user_id: i64) {
        self.users.push(user_id);
    }

    async fn finish<T>(mut self, outcome: Result<T, Box<dyn std::any::Any + Send>>) -> T {
        let errors = self.release().await;
        match outcome {
            Ok(value) => {
                assert!(
                    errors.is_empty(),
                    "admission teardown failed: {}",
                    errors.join("; ")
                );
                value
            }
            Err(payload) => {
                if !errors.is_empty() {
                    eprintln!("admission teardown also failed: {}", errors.join("; "));
                }
                std::panic::resume_unwind(payload)
            }
        }
    }

    async fn release(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for item in std::mem::take(&mut self.claims) {
            let sealed = std::panic::AssertUnwindSafe(seal_test_execution(self.db, &item))
                .catch_unwind()
                .await;
            match sealed {
                Ok(()) => {}
                Err(payload) => {
                    let detail = payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| {
                            payload
                                .downcast_ref::<&str>()
                                .map(|text| (*text).to_owned())
                        })
                        .unwrap_or_else(|| "non-string panic".to_owned());
                    errors.push(format!(
                        "seal grant task_id={} item_id={} panicked: {detail}",
                        item.task_id, item.id
                    ));
                }
            }
        }
        for task_id in std::mem::take(&mut self.tasks) {
            if let Err(error) = try_delete_task(self.db, task_id).await {
                errors.push(error);
            }
        }
        for user_id in std::mem::take(&mut self.users) {
            if let Err(error) = try_delete_user(self.db, user_id).await {
                errors.push(error);
            }
        }
        if let Err(error) = self.restore_policy().await {
            errors.push(error);
        }
        errors
    }

    async fn restore_policy(&self) -> Result<(), String> {
        self.db
            .execution_queue()
            .await
            .map_err(|error| format!("load the execution queue: {error}"))?
            .set_policy(self.previous_policy)
            .await
            .map_err(|error| format!("restore the captured queue policy: {error}"))
    }
}

async fn try_delete_task(db: &Database, task_id: Uuid) -> Result<(), String> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("clean up task items: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("clean up task: {error}"))?;
    Ok(())
}

async fn try_delete_user(db: &Database, user_id: i64) -> Result<(), String> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("clean up idempotency keys: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("clean up user: {error}"))?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_admissions_do_not_oversell_the_parent_capacity() {
    let db = connect().await;
    let _guard = ADMISSION_LOCK.lock().await;
    let mut scope = AdmissionScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let mut seeded = Vec::new();
        for index in 0..3 {
            let (task_id, items) = seed_task(
                &db,
                user_id,
                &[json!({"external_id": format!("concurrent-{index}")})],
                &format!("concurrent-{index}"),
            )
            .await;
            scope.track_task(task_id);
            seeded.push((task_id, items));
        }
        let fixture_tasks: Vec<Uuid> = seeded.iter().map(|(task_id, _)| *task_id).collect();
        let fixture_items: Vec<Uuid> = seeded
            .iter()
            .flat_map(|(_, item_ids)| item_ids.iter().copied())
            .collect();

        let capacity = 2;
        db.apply_execution_queue_policy(capacity)
            .await
            .expect("set the global queue policy");

        // Eight concurrent claimers, as several replicas would run them. The
        // durable queue policy serializes the capacity check, so the total never
        // exceeds it.
        let mut claims = Vec::new();
        for _ in 0..8 {
            let db = db.clone();
            claims.push(tokio::spawn(async move { db.claim_items().await }));
        }
        let mut admitted = Vec::new();
        for claim in claims {
            let items = claim.await.expect("join claim").expect("claim succeeds");
            admitted.extend(scope.own_claims(items, &fixture_tasks, &fixture_items));
        }

        assert_eq!(
            admitted.len(),
            capacity,
            "the global queue policy admits exactly its capacity, never more"
        );
        let admitted_tasks = admitted
            .iter()
            .map(|item| item.task_id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            admitted_tasks.len(),
            capacity,
            "each admitted attempt belongs to a distinct parent"
        );

        // The queued work is untouched: it never started and holds no attempt.
        let queued = seeded
            .iter()
            .find(|entry| !admitted_tasks.contains(&entry.0))
            .expect("one seeded parent stays queued");
        let (queued_task, queued_items) = queued;
        assert_eq!(item_status(&db, queued_items[0]).await, "queued");
        assert_eq!(
            item_attempt_count(&db, queued_items[0]).await,
            0,
            "a capped claim never increments a queued item"
        );
        assert!(
            parent_started_at(&db, *queued_task).await.is_none(),
            "queued work has not started, so the parent has no start time"
        );

        // Releasing the admitted attempts admits the queued parent on the next
        // claim; a merely nonempty result would not prove that.
        for item in admitted {
            finish(&db, item).await;
        }
        let after = scope.own_claims(
            db.claim_items().await.expect("claim after release"),
            &fixture_tasks,
            &fixture_items,
        );
        let queued_claim = after
            .iter()
            .find(|item| item.task_id == *queued_task && item.id == queued_items[0])
            .expect("the queued parent becomes claimable once capacity is released");
        assert_eq!(queued_claim.attempt_count, 1, "the released slot admits it");
        let admitted_queued = after
            .into_iter()
            .find(|item| item.task_id == *queued_task)
            .expect("the queued claim is owned");
        assert!(
            parent_started_at(&db, *queued_task).await.is_some(),
            "the first real claim stamps the parent start time"
        );
        finish(&db, admitted_queued).await;
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}
