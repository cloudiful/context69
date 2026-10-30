//! Selection snapshot regressions (issue 1130 P1).
//!
//! `GET /v1/tasks/selection` reads the ids of every task that matches the
//! current filters, independent of pagination and sorting. These tests pin the
//! membership contract the bulk actions rely on: the snapshot holds every
//! matching task across pages, it holds only the caller's visible tasks, the
//! view and status filters narrow it exactly like the list does, and the read
//! stops at the cap instead of returning an unbounded set.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::db::Database;
use context69_contracts::tasks::{TaskListView, TaskSelectionQuery, TaskStatus};
use sqlx::Row;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The cap the service reads with: the snapshot limit plus one row, so an
/// oversized matching set is detected instead of being truncated.
const CAP_PLUS_ONE: i64 = 50_001;

fn selection_query(view: TaskListView) -> TaskSelectionQuery {
    TaskSelectionQuery {
        query: None,
        kind: None,
        status: None,
        view,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
    }
}

async fn seed_user(db: &Database, tag: &str) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("selection-{tag}-{}", Uuid::new_v4()))
    .bind("Selection Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

async fn seed_group(db: &Database, parent: Option<(i64, String)>) -> (i64, String) {
    let key = format!("selection-{}", Uuid::new_v4().simple());
    let full_path = match &parent {
        None => key.clone(),
        Some((_, parent_path)) => format!("{parent_path}/{key}"),
    };
    let parent_id: Option<i64> = parent.map(|(id, _)| id);
    let id: i64 = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, full_path, visibility, kind, parent_group_id) \
         VALUES ($1, $1, $2, 'private', 'shared', $3) RETURNING id",
    )
    .bind(&key)
    .bind(&full_path)
    .bind(parent_id)
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id");
    (id, full_path)
}

async fn add_member(db: &Database, group_id: i64, user_id: i64) {
    sqlx::query(
        "INSERT INTO context69.group_memberships (group_id, user_id, role) \
         VALUES ($1, $2, 'viewer') ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(user_id)
    .execute(db.pool())
    .await
    .expect("add member");
}

async fn insert_task(
    db: &Database,
    user_id: i64,
    group_id: Option<i64>,
    kind: &str,
    status: &str,
    group_path: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, group_id, kind, status, origin, group_path, total_count, queued_count, stage) \
         VALUES ($1, $2, $3, $4, $5, 'manual', $6, 1, 1, 'processing')",
    )
    .bind(id)
    .bind(user_id)
    .bind(group_id)
    .bind(kind)
    .bind(status)
    .bind(group_path)
    .execute(db.pool())
    .await
    .expect("insert task");
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status, stage) \
         VALUES ($1, $2, 0, $3, 'queued', 'processing')",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .bind(serde_json::json!({ "external_id": group_path }))
    .execute(db.pool())
    .await
    .expect("insert item");
    id
}

async fn trash_task(db: &Database, task_id: Uuid) {
    sqlx::query("UPDATE context69.tasks SET deleted_at = now() WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("trash task");
}

async fn snapshot_ids(db: &Database, user_id: i64, query: &TaskSelectionQuery) -> Vec<Uuid> {
    db.list_task_selection(user_id, query, CAP_PLUS_ONE)
        .await
        .expect("list task selection")
        .into_iter()
        .map(|(task_id, _)| task_id)
        .collect()
}

async fn cleanup(db: &Database, task_ids: &[Uuid], group_ids: &[i64], user_ids: &[i64]) {
    for task_id in task_ids {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("clean items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("clean task");
    }
    for group_id in group_ids {
        sqlx::query("DELETE FROM context69.group_memberships WHERE group_id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean membership");
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean group");
    }
    for user_id in user_ids {
        sqlx::query("DELETE FROM context69.users WHERE id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("clean user");
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn selection_covers_every_matching_task_and_excludes_other_users() {
    let url = context69_test_support::accessor::database_url();
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let owner = seed_user(&db, "owner").await;
    let outsider = seed_user(&db, "outsider").await;
    let tag = Uuid::new_v4().simple().to_string();

    let mut owned = Vec::new();
    for index in 0..3 {
        owned.push(
            insert_task(
                &db,
                owner,
                None,
                "text_batch",
                "queued",
                &format!("test/selection-{tag}-{index}"),
            )
            .await,
        );
    }
    let foreign = insert_task(
        &db,
        outsider,
        None,
        "text_batch",
        "queued",
        &format!("test/selection-{tag}-foreign"),
    )
    .await;

    let query = selection_query(TaskListView::Processing);
    let ids = snapshot_ids(&db, owner, &query).await;

    for task_id in &owned {
        assert!(
            ids.contains(task_id),
            "the snapshot must hold every own task, not only the current page"
        );
    }
    assert!(
        !ids.contains(&foreign),
        "the snapshot must not hold another user's task"
    );
    assert_eq!(
        ids.len(),
        owned.len(),
        "the snapshot must hold exactly the own matching tasks"
    );

    cleanup(
        &db,
        &[owned[0], owned[1], owned[2], foreign],
        &[],
        &[owner, outsider],
    )
    .await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn selection_applies_the_view_and_status_filters() {
    let url = context69_test_support::accessor::database_url();
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let owner = seed_user(&db, "filters").await;
    let tag = Uuid::new_v4().simple().to_string();

    let queued = insert_task(
        &db,
        owner,
        None,
        "text_batch",
        "queued",
        &format!("test/selection-{tag}-queued"),
    )
    .await;
    let failed = insert_task(
        &db,
        owner,
        None,
        "url_batch",
        "failed",
        &format!("test/selection-{tag}-failed"),
    )
    .await;
    let succeeded = insert_task(
        &db,
        owner,
        None,
        "text_batch",
        "succeeded",
        &format!("test/selection-{tag}-succeeded"),
    )
    .await;
    let trashed = insert_task(
        &db,
        owner,
        None,
        "text_batch",
        "succeeded",
        &format!("test/selection-{tag}-trashed"),
    )
    .await;
    trash_task(&db, trashed).await;

    let processing = snapshot_ids(&db, owner, &selection_query(TaskListView::Processing)).await;
    assert!(processing.contains(&queued));
    assert!(processing.contains(&failed));
    assert!(
        !processing.contains(&succeeded),
        "a succeeded task does not belong to the processing view"
    );
    assert!(
        !processing.contains(&trashed),
        "a trashed task does not belong to the processing view"
    );

    let completed = snapshot_ids(&db, owner, &selection_query(TaskListView::Completed)).await;
    assert!(completed.contains(&succeeded));
    assert!(
        !completed.contains(&queued) && !completed.contains(&failed),
        "the completed view holds only succeeded tasks"
    );
    assert!(!completed.contains(&trashed));

    let trash = snapshot_ids(&db, owner, &selection_query(TaskListView::Trash)).await;
    assert_eq!(trash, vec![trashed]);

    let mut failed_only = selection_query(TaskListView::Processing);
    failed_only.status = Some(TaskStatus::Failed);
    let failed_ids = snapshot_ids(&db, owner, &failed_only).await;
    assert_eq!(failed_ids, vec![failed]);

    let mut url_only = selection_query(TaskListView::Processing);
    url_only.kind = Some(context69_contracts::tasks::TaskKind::UrlBatch);
    let url_ids = snapshot_ids(&db, owner, &url_only).await;
    assert_eq!(url_ids, vec![failed]);

    cleanup(&db, &[queued, failed, succeeded, trashed], &[], &[owner]).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn selection_follows_the_group_visibility_of_the_list() {
    let url = context69_test_support::accessor::database_url();
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let owner = seed_user(&db, "group-owner").await;
    let member = seed_user(&db, "group-member").await;
    let outsider = seed_user(&db, "group-outsider").await;
    let (parent, parent_path) = seed_group(&db, None).await;
    let (child, _) = seed_group(&db, Some((parent, parent_path))).await;
    add_member(&db, parent, member).await;

    let group_task = insert_task(
        &db,
        owner,
        Some(child),
        "text_batch",
        "queued",
        "test/selection-group",
    )
    .await;
    let personal = insert_task(
        &db,
        owner,
        None,
        "text_batch",
        "queued",
        "test/selection-personal",
    )
    .await;

    let member_ids = snapshot_ids(&db, member, &selection_query(TaskListView::Processing)).await;
    assert!(
        member_ids.contains(&group_task),
        "an inheriting group member must see the group task in the snapshot"
    );
    assert!(
        !member_ids.contains(&personal),
        "a member must not see another user's personal task in the snapshot"
    );

    let outsider_ids =
        snapshot_ids(&db, outsider, &selection_query(TaskListView::Processing)).await;
    assert!(!outsider_ids.contains(&group_task));
    assert!(!outsider_ids.contains(&personal));

    cleanup(
        &db,
        &[group_task, personal],
        &[child, parent],
        &[owner, member, outsider],
    )
    .await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn selection_stops_at_the_read_limit() {
    let url = context69_test_support::accessor::database_url();
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let owner = seed_user(&db, "limit").await;
    let tag = Uuid::new_v4().simple().to_string();

    let mut owned = Vec::new();
    for index in 0..3 {
        owned.push(
            insert_task(
                &db,
                owner,
                None,
                "text_batch",
                "queued",
                &format!("test/selection-{tag}-{index}"),
            )
            .await,
        );
    }

    // The limit is the only bound on the read: the statement stops there, so a
    // caller can detect an oversized set by reading one row past the cap.
    let bounded = db
        .list_task_selection(owner, &selection_query(TaskListView::Processing), 2)
        .await
        .expect("list task selection");
    assert_eq!(
        bounded.len(),
        2,
        "the read must stop at the requested limit"
    );

    let full = snapshot_ids(&db, owner, &selection_query(TaskListView::Processing)).await;
    assert_eq!(full.len(), 3);

    cleanup(&db, &owned, &[], &[owner]).await;
}
