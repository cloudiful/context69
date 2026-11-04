//! Shared fixtures for the Phase 3 claim-hot-path split tests.

use cloudiful_tasks::{Queue, TaskId};
use context69::db::Database;
use context69_test_support::execution_cleanup::{
    FixtureExecutionScope, quiesce_fixture_executions,
};
use context69_test_support::library_policy::LibraryPolicyBaseline;
use uuid::Uuid;

/// `claim_items` is a global dispatcher primitive, and `maintain_claim_state`
/// operates on the same shared scratch database. Serialise the file so
/// concurrent runs do not observe each other's partial state.
pub static FAST_PATH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("fast-path-test-{}", Uuid::new_v4()),
        "Fast Path Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
}

pub async fn insert_file(db: &Database, status: &str) -> (Uuid, i64) {
    let group_id = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("fast-path-{}", Uuid::new_v4()),
        "Fast Path Test Group",
        format!("test/fast-path-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test group");
    let id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_processing_file.sql",
        id,
        group_id,
        format!("file-{id}.txt"),
        status
    )
    .execute(db.pool())
    .await
    .expect("insert test file");
    (id, group_id)
}

pub async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
    if let Err(error) = try_cleanup_task(db, task_id, user_id).await {
        panic!("{error}");
    }
}

async fn try_cleanup_task(db: &Database, task_id: Uuid, user_id: i64) -> Result<(), String> {
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
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("clean up idempotency keys: {error}"))?;
    Ok(())
}

pub async fn cleanup_file(db: &Database, file_id: Uuid, group_id: i64) {
    sqlx::query_file!("tests/sql/task_execution/delete_file_by_id.sql", file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

pub async fn cleanup_user(db: &Database, user_id: i64) {
    if let Err(error) = try_cleanup_user(db, user_id).await {
        panic!("{error}");
    }
}

async fn try_cleanup_user(db: &Database, user_id: i64) -> Result<(), String> {
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("clean up user: {error}"))?;
    Ok(())
}

/// Seals the exact live library attempt a test owns before its domain fixture
/// is deleted, so the global queue capacity reflects real work rather than test
/// teardown artifacts.
///
/// The cleanup uses the captured grant itself. It is not a substitute for
/// `finish_task_item`: a deliberately corrupted domain lease would make that
/// transition roll the library seal back, so the library attempt is sealed
/// directly through the queue.
pub async fn seal_test_execution(db: &Database, item: &context69::db::ClaimedItem) {
    if let Err(error) = try_seal_test_execution(db, item).await {
        panic!("{error}");
    }
}

async fn try_seal_test_execution(
    db: &Database,
    item: &context69::db::ClaimedItem,
) -> Result<(), String> {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let queue = db
        .execution_queue()
        .await
        .map_err(|error| format!("load the execution queue: {error}"))?;
    let result = queue
        .fail(
            &execution.attempt,
            &cloudiful_tasks::Failure::permanent(Some(
                serde_json::json!({"reason": "context69_test_cleanup"}),
            )),
        )
        .await;
    match result {
        Ok(_) => Ok(()),
        Err(
            error @ (cloudiful_tasks::Error::LeaseLost { .. }
            | cloudiful_tasks::Error::InvalidTaskState { .. }),
        ) => {
            let attempts = queue
                .attempts(execution.attempt.task_id(), 16)
                .await
                .map_err(|error| format!("read the captured task attempts: {error}"))?;
            let sealed = attempts.iter().any(|attempt| {
                attempt.id() == execution.attempt.id() && attempt.status().is_sealed()
            });
            if sealed {
                Ok(())
            } else {
                Err(format!(
                    "test cleanup must seal the captured attempt or confirm it already sealed: {error}"
                ))
            }
        }
        Err(error) => Err(format!(
            "test cleanup could not seal the captured attempt: {error}"
        )),
    }
}

/// Crashes the worker that holds the item.
///
/// The authoritative lease is the library attempt: the crash expires that
/// attempt, and the domain mirror of it expires with it. Backdating only the
/// domain mirror would not be a library crash, and no takeover would be
/// authorized by it.
pub async fn crash_holder(db: &Database, item: &context69::db::ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    sqlx::query_file!(
        "tests/sql/task_execution/expire_library_attempt_by_id.sql",
        execution.attempt_id()
    )
    .execute(db.pool())
    .await
    .expect("expire the library attempt");
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
}

/// Assert every grant a claim call returned belongs to this test's known
/// fixtures and return them all.
///
/// A claim is global, so a returned item this test did not seed is an
/// unexpected live grant. Silently dropping it with `find`/`all` would both leak
/// capacity and hide the real owner, so the mismatch fails with the returned and
/// expected identities instead.
pub fn own_claims(
    returned: Vec<context69::db::ClaimedItem>,
    fixture_tasks: &[Uuid],
    fixture_items: &[Uuid],
) -> Vec<context69::db::ClaimedItem> {
    assert_owned_claims(&returned, fixture_tasks, fixture_items);
    returned
}

fn assert_owned_claims(
    returned: &[context69::db::ClaimedItem],
    fixture_tasks: &[Uuid],
    fixture_items: &[Uuid],
) {
    for item in returned {
        assert!(
            fixture_tasks.contains(&item.task_id) && fixture_items.contains(&item.id),
            "claim returned a grant outside this test's fixtures: \
             task_id={} item_id={} fixture_tasks={fixture_tasks:?} fixture_items={fixture_items:?}",
            item.task_id,
            item.id
        );
    }
}

/// Failure-safe teardown for the capacity cases that share one reusable
/// database.
///
/// A capacity case changes global state: it holds live library grants and it
/// rewrites the durable queue policy. When an assertion fails before its
/// explicit cleanup, that state outlives the test and poisons every later case
/// that shares the database. This scope records only the tasks, grants, probe
/// queues and users the test created, releases them through the same helpers as
/// the passing path, and restores the policy observed before the test ran.
pub struct CapacityScope<'a> {
    db: &'a Database,
    baseline: LibraryPolicyBaseline,
    claims: Vec<context69::db::ClaimedItem>,
    tasks: Vec<(Uuid, i64)>,
    users: Vec<i64>,
    probes: Vec<(Queue, TaskId)>,
    execution_tasks: Vec<Uuid>,
}

impl<'a> CapacityScope<'a> {
    /// Capture the queue policy this test must restore.
    pub async fn new(db: &'a Database) -> Self {
        let queue = db
            .execution_queue()
            .await
            .expect("load the execution queue");
        let baseline = LibraryPolicyBaseline::capture(queue)
            .await
            .expect("capture the execution policy baseline");
        Self {
            db,
            baseline,
            claims: Vec::new(),
            tasks: Vec::new(),
            users: Vec::new(),
            probes: Vec::new(),
            execution_tasks: Vec::new(),
        }
    }

    /// Assert ownership of every returned grant, track all of them and return
    /// them so the test still selects the one it asserts on.
    pub fn own_claims(
        &mut self,
        returned: Vec<context69::db::ClaimedItem>,
        fixture_tasks: &[Uuid],
        fixture_items: &[Uuid],
    ) -> Vec<context69::db::ClaimedItem> {
        assert_owned_claims(&returned, fixture_tasks, fixture_items);
        self.claims.extend(returned.iter().cloned());
        returned
    }

    pub fn track_task(&mut self, task_id: Uuid, user_id: i64) {
        self.tasks.push((task_id, user_id));
    }

    pub fn track_user(&mut self, user_id: i64) {
        self.users.push(user_id);
    }

    pub fn track_probe(&mut self, queue: &Queue, task_id: TaskId) {
        self.probes.push((queue.clone(), task_id));
    }

    /// Adds one fixture parent task to the exact public quiesce scope during
    /// teardown. Its domain rows are removed by a separately tracked user.
    pub fn track_execution_task(&mut self, task_id: Uuid) {
        self.execution_tasks.push(task_id);
    }

    /// Await every tracked release step, then surface the body panic or a
    /// teardown error. Nothing is spawned, so teardown is complete before the
    /// original failure continues.
    pub async fn finish<T>(mut self, outcome: Result<T, Box<dyn std::any::Any + Send>>) -> T {
        let errors = self.release().await;
        match outcome {
            Ok(value) => {
                assert!(
                    errors.is_empty(),
                    "capacity teardown failed: {}",
                    errors.join("; ")
                );
                value
            }
            Err(payload) => {
                if !errors.is_empty() {
                    eprintln!("capacity teardown also failed: {}", errors.join("; "));
                }
                std::panic::resume_unwind(payload)
            }
        }
    }

    async fn release(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for item in std::mem::take(&mut self.claims) {
            if let Err(error) = try_seal_test_execution(self.db, &item).await {
                errors.push(error);
            }
        }
        for (queue, task_id) in std::mem::take(&mut self.probes) {
            if let Err(error) = queue.request_cancel(task_id).await {
                errors.push(format!("cancel probe task {task_id}: {error}"));
            }
        }
        let mut execution_tasks = std::mem::take(&mut self.execution_tasks);
        execution_tasks.extend(self.tasks.iter().map(|(task_id, _)| *task_id));
        execution_tasks.sort_unstable();
        execution_tasks.dedup();
        if let Err(error) = quiesce_fixture_executions(
            self.db.pool(),
            FixtureExecutionScope::Tasks(&execution_tasks),
        )
        .await
        {
            errors.push(format!("quiesce fixture executions: {error}"));
        }
        for (task_id, user_id) in std::mem::take(&mut self.tasks) {
            if let Err(error) = try_cleanup_task(self.db, task_id, user_id).await {
                errors.push(error);
            }
        }
        for user_id in std::mem::take(&mut self.users) {
            if let Err(error) = try_cleanup_user(self.db, user_id).await {
                errors.push(error);
            }
        }
        if let Err(error) = self.restore_policy().await {
            errors.push(error);
        }
        errors
    }

    async fn restore_policy(&self) -> Result<(), String> {
        let queue = self
            .db
            .execution_queue()
            .await
            .map_err(|error| format!("load the execution queue: {error}"))?;
        self.baseline
            .restore(self.db.pool(), queue)
            .await
            .map_err(|error| format!("restore the captured queue policy: {error}"))
    }
}
