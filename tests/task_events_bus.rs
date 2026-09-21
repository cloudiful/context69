//! Task event bus integration tests (issue 405 Task E1).
//!
//! Proves PG NOTIFY is not falsified: triggers on `tasks`/`task_items`
//! fan out through `PgListener -> broadcast` with a 4-field payload, the
//! heartbeat throttle suppresses storms, and LISTEN recovers after a
//! terminated backend.
//!
//! SQL-only path coverage (one case each):
//! - cancel via `Database::cancel_task` (`cancel.sql` + `cancel_items.sql`)
//! - trash via `Database::trash_task` (`trash.sql`)
//! - restore via `Database::restore_task` (`restore.sql`)
//! - clear via `Database::clear_user_task_history` (`clear_user_task_history.sql` DELETE)
//! - `maintain_claim_state` (`maintain_claim_state.sql`)
//!
//! These tests run only when `CONTEXT69_TEST_DATABASE_URL` points to a
//! scratch database (migrations applied automatically). Skipped otherwise.

use std::time::Duration;

use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::services::tasks::events::{TASK_EVENTS_CHANNEL, TaskEvent};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static BUS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("bus-test-{}", Uuid::new_v4()))
    .bind("Bus Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn create_text_task(
    db: &Database,
    user_id: i64,
    tag: &str,
    count: usize,
) -> (Uuid, Vec<Uuid>) {
    let payloads: Vec<serde_json::Value> = (0..count)
        .map(|index| json!({ "external_id": format!("{tag}-{index}") }))
        .collect();
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/bus"),
            source_key: None,
            payloads: &payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: tag,
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("clean up task items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("clean up task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

async fn spawn_bus_and_wait(db: &Database) -> tokio::sync::broadcast::Sender<TaskEvent> {
    let sender =
        context69::services::tasks::events::spawn_task_event_listener(db.pool().clone());
    // The hub LISTENs asynchronously; wait for the subscription before the
    // operation under test so its NOTIFY is not lost to a pre-LISTEN race.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    sender
}

async fn collect_events_for(
    receiver: &mut tokio::sync::broadcast::Receiver<TaskEvent>,
    task_id: Uuid,
    timeout: Duration,
) -> Vec<TaskEvent> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut matched = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(event)) => {
                if event.task_id == task_id {
                    matched.push(event);
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            _ => break,
        }
    }
    matched
}

async fn wait_for_event_matching(
    receiver: &mut tokio::sync::broadcast::Receiver<TaskEvent>,
    task_id: Uuid,
    predicate: impl Fn(&TaskEvent) -> bool,
    timeout: Duration,
) -> Option<TaskEvent> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(event)) => {
                if event.task_id == task_id && predicate(&event) {
                    return Some(event);
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            _ => return None,
        }
    }
}

#[tokio::test]
async fn insert_receives_task_and_item_events() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping insert event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();
    let user_id = seed_test_user(&db).await;

    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-insert-hash", 1).await;
    assert_eq!(item_ids.len(), 1);

    let task_level = wait_for_event_matching(
        &mut receiver,
        task_id,
        |event| event.item_id.is_none(),
        Duration::from_secs(10),
    )
    .await;
    assert!(
        task_level.is_some(),
        "insert must emit a task-level event (item_id None) for {task_id}"
    );
    let item_level = wait_for_event_matching(
        &mut receiver,
        task_id,
        |event| event.item_id == Some(item_ids[0]),
        Duration::from_secs(10),
    )
    .await;
    assert!(
        item_level.is_some(),
        "insert must emit an item-level event for {}, got task event {task_level:?}",
        item_ids[0]
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn update_receives_item_event() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping update event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-update-hash", 1).await;
    // Subscribe after setup so only the UPDATE under test is observed.
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() + interval '5 minutes', started_at = coalesce(started_at, now()), \
         updated_at = now() WHERE id = $1",
    )
    .bind(item_ids[0])
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("mark item running");

    let event = wait_for_event_matching(
        &mut receiver,
        task_id,
        |event| event.item_id == Some(item_ids[0]) && event.status == "running",
        Duration::from_secs(10),
    )
    .await;
    assert!(
        event.is_some(),
        "item UPDATE to running must emit an event for item {}",
        item_ids[0]
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn payload_is_four_fields_and_small() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping payload ceiling test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let mut listener = sqlx::postgres::PgListener::connect(&url)
        .await
        .expect("connect listener");
    listener
        .listen(TASK_EVENTS_CHANNEL)
        .await
        .expect("LISTEN task_events");

    let (task_id, _items) = create_text_task(&db, user_id, "bus-payload-hash", 1).await;

    let mut payload_text: Option<String> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, listener.recv()).await {
            Ok(Ok(notification)) => {
                let text = notification.payload().to_string();
                if text.contains(&task_id.to_string()) {
                    payload_text = Some(text);
                    break;
                }
            }
            _ => break,
        }
    }
    let payload_text = payload_text.expect("must receive a payload for the new task");
    assert!(
        payload_text.len() < 8000,
        "pg_notify payload must stay below 8000 bytes, got {}",
        payload_text.len()
    );
    assert!(
        payload_text.len() < 1000,
        "4-field task event should be tiny, got {} bytes: {payload_text}",
        payload_text.len()
    );
    let value: serde_json::Value =
        serde_json::from_str(&payload_text).expect("payload must be JSON");
    let object = value.as_object().expect("payload must be an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["item_id", "status", "task_id", "updated_at"],
        "payload must be limited to task_id/item_id/status/updated_at, got {payload_text}"
    );
    // Must parse through the hub type as well.
    let event = TaskEvent::parse_payload(&payload_text).expect("hub must parse payload");
    assert_eq!(event.task_id, task_id);

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn heartbeat_update_is_throttled() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping heartbeat throttle test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-heartbeat-hash", 1).await;
    let item_id = item_ids[0];

    // Put the item into running with a lease so heartbeat_item.sql shape applies.
    let lease = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() + interval '5 minutes', updated_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease)
    .execute(db.pool())
    .await
    .expect("mark running");
    // Subscribe after setup so only heartbeat/revocation are observed.
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    // Pure heartbeat: only lease_until + updated_at change (mirrors heartbeat_item.sql).
    sqlx::query(
        "UPDATE context69.task_items SET lease_until = now() + interval '5 minutes', \
         updated_at = now() WHERE id = $1 AND lease_token = $2 AND status = 'running'",
    )
    .bind(item_id)
    .bind(lease)
    .execute(db.pool())
    .await
    .expect("heartbeat");

    let heartbeat_events =
        collect_events_for(&mut receiver, task_id, Duration::from_secs(2)).await;
    assert!(
        heartbeat_events.is_empty(),
        "pure heartbeat (lease_until/updated_at only) must be throttled, got {heartbeat_events:?}"
    );

    // A business change on the same row must still notify (lease revocation
    // keeps status running but changes lease_token, like expired_items).
    sqlx::query(
        "UPDATE context69.task_items SET lease_token = NULL, lease_until = NULL, \
         updated_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("revoke lease");
    let revoked = wait_for_event_matching(
        &mut receiver,
        task_id,
        |event| event.item_id == Some(item_id),
        Duration::from_secs(10),
    )
    .await;
    assert!(
        revoked.is_some(),
        "lease revocation (lease_token change) must still emit despite same status"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn cancel_path_emits_event() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping cancel event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _items) = create_text_task(&db, user_id, "bus-cancel-hash", 1).await;
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    // cancel.sql + cancel_items.sql + recompute.sql (SQL-only path via Database).
    let cancelled = db
        .cancel_task(task_id, user_id)
        .await
        .expect("cancel task");
    assert!(cancelled, "cancel must succeed for a queued task");

    let event = wait_for_event_matching(
        &mut receiver,
        task_id,
        |event| event.status == "cancelled",
        Duration::from_secs(10),
    )
    .await;
    assert!(
        event.is_some(),
        "cancel (cancel.sql/cancel_items.sql) must emit a cancelled event for {task_id}"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn trash_path_emits_event() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping trash event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-trash-hash", 1).await;
    // Drive to terminal so trash.sql matches (status succeeded).
    sqlx::query("UPDATE context69.task_items SET status = 'succeeded', finished_at = now() WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("finish item");
    db.recompute_task(task_id).await.expect("recompute");
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    let trashed = db.trash_task(task_id).await.expect("trash task");
    assert!(trashed, "trash must move a terminal task");

    let event = wait_for_event_matching(
        &mut receiver,
        task_id,
        |_| true,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        event.is_some(),
        "trash (trash.sql deleted_at update) must emit for {task_id}"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn restore_path_emits_event() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping restore event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-restore-hash", 1).await;
    sqlx::query("UPDATE context69.task_items SET status = 'succeeded', finished_at = now() WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("finish item");
    db.recompute_task(task_id).await.expect("recompute");
    assert!(db.trash_task(task_id).await.expect("trash"));
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    let restored = db.restore_task(task_id).await.expect("restore task");
    assert!(restored, "restore must clear the trash marker");

    let event = wait_for_event_matching(
        &mut receiver,
        task_id,
        |_| true,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        event.is_some(),
        "restore (restore.sql deleted_at clear) must emit for {task_id}"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn clear_path_emits_delete_event() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping clear event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-clear-hash", 1).await;
    sqlx::query("UPDATE context69.task_items SET status = 'succeeded', finished_at = now() WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("finish item");
    // Drive the parent to succeeded so the completed view matches.
    sqlx::query("UPDATE context69.tasks SET status = 'succeeded', finished_at = now() WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("mark task succeeded");
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    // clear_user_task_history.sql DELETE (SQL-only, cascades to items).
    let deleted = db
        .clear_user_task_history(user_id, "completed")
        .await
        .expect("clear history");
    assert_eq!(deleted, 1, "clear must delete the succeeded row");

    let event = wait_for_event_matching(
        &mut receiver,
        task_id,
        |_| true,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        event.is_some(),
        "clear (DELETE FROM tasks) must emit a delete event for {task_id}"
    );

    // Row is gone; only the user remains to clean up.
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn maintain_claim_state_path_emits_event() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping maintain_claim_state event test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = create_text_task(&db, user_id, "bus-maintain-hash", 1).await;
    let item_id = item_ids[0];
    // Exhausted predicate: queued with attempt_count >= 5 on a queued task.
    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 5, status = 'queued', \
         next_attempt_at = NULL WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("set exhausted predicate");
    sqlx::query("UPDATE context69.tasks SET status = 'queued' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("set task queued");
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 1,
        "maintenance must exhaust the fixture item"
    );

    let event = wait_for_event_matching(
        &mut receiver,
        task_id,
        |event| event.status == "failed",
        Duration::from_secs(10),
    )
    .await;
    assert!(
        event.is_some(),
        "maintain_claim_state (exhausted -> failed) must emit a failed event for {task_id}"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn listener_recovers_after_backend_terminate() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping reconnect test");
        return;
    };
    let _guard = BUS_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // Hub path (broadcast) must survive a terminated PG backend via
    // PgListener's transparent reconnect + re-LISTEN.
    let sender = spawn_bus_and_wait(&db).await;
    let mut receiver = sender.subscribe();

    let (first_task, _) = create_text_task(&db, user_id, "bus-reconnect-first-hash", 1).await;
    let first = wait_for_event_matching(
        &mut receiver,
        first_task,
        |_| true,
        Duration::from_secs(10),
    )
    .await;
    assert!(
        first.is_some(),
        "listener must receive events before the terminate"
    );

    // Terminate every other backend on this database (includes the
    // listener's dedicated connection). The pool reconnects on next use and
    // the listener re-issues LISTEN transparently.
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() AND pid <> pg_backend_pid()",
    )
    .execute(db.pool())
    .await
    .expect("terminate other backends");
    // Give the listener a moment to reconnect and re-LISTEN.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let (second_task, _) = create_text_task(&db, user_id, "bus-reconnect-second-hash", 1).await;
    let second = wait_for_event_matching(
        &mut receiver,
        second_task,
        |_| true,
        Duration::from_secs(15),
    )
    .await;
    assert!(
        second.is_some(),
        "listener must recover LISTEN after backend terminate and receive for {second_task}"
    );

    cleanup_task(&db, first_task, user_id).await;
    // Second task shares the same user; manual cleanup for its rows then the user.
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(second_task)
        .execute(db.pool())
        .await
        .expect("clean up second items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(second_task)
        .execute(db.pool())
        .await
        .expect("clean up second task");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}
