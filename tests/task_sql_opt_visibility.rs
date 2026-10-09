//! Issue 734 visibility regressions for the optimized `list.sql`/`count.sql`.
//!
//! The inherited accessible groups are now computed once outside the
//! task-correlated EXISTS. These tests pin the preserved behavior: owner,
//! nonmember, direct member, descendant, and overlapping membership visibility,
//! plus list/count equivalence with filters and pagination.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database
//! (migrations are applied automatically). They are skipped otherwise.

use sqlx::Row;
use uuid::Uuid;

use context69::db::{Database, TaskCountFilter, TaskListFilter};

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const LIST_SQL: &str = include_str!("../src/sql/db/tasks/list.sql");
const COUNT_SQL: &str = include_str!("../src/sql/db/tasks/count.sql");

#[test]
fn optimized_visibility_computes_groups_once_without_duplicates() {
    for sql in [LIST_SQL, COUNT_SQL] {
        assert!(
            sql.contains("WITH RECURSIVE accessible_groups"),
            "visibility must compute accessible groups once"
        );
        assert!(
            sql.contains("FROM accessible_groups ag"),
            "tasks must filter against the hoisted set"
        );
        assert!(
            !sql.contains("inherited_groups"),
            "per-row inherited_groups CTE must be gone"
        );
    }
    assert!(
        LIST_SQL.contains("WITH RECURSIVE accessible_groups") && LIST_SQL.contains("page AS"),
        "list must keep the page-first shape"
    );
    assert!(
        !COUNT_SQL.contains("LATERAL"),
        "count must stay metadata-free"
    );
}

fn test_db_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_user(db: &Database, tag: &str) -> i64 {
    sqlx::query("INSERT INTO context69.users (login_name, display_name, password_hash) VALUES ($1, $2, $3) RETURNING id")
        .bind(format!("sql-opt-{tag}-{}", Uuid::new_v4()))
        .bind("Sql Opt Test")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
        .get("id")
}

async fn seed_group(db: &Database, parent: Option<(i64, String)>) -> (i64, String) {
    let key = format!("sql-opt-{}", Uuid::new_v4().simple());
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
    // One queued item so payload search has a row to match when needed.
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

async fn list_ids(db: &Database, user_id: i64, view: &str, limit: i64, offset: i64) -> Vec<Uuid> {
    db.list_tasks(TaskListFilter {
        user_id,
        query: None,
        kind: None,
        status: None,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        sort_by: None,
        sort_direction: None,
        limit,
        offset,
        view,
    })
    .await
    .expect("list tasks")
    .into_iter()
    .map(|t| t.id)
    .collect()
}

async fn count(db: &Database, user_id: i64, view: &str) -> i64 {
    db.count_tasks(TaskCountFilter {
        user_id,
        query: None,
        kind: None,
        status: None,
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        view,
    })
    .await
    .expect("count tasks")
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
async fn visibility_covers_owner_member_descendant_and_nonmember() {
    let Some(url) = test_db_url() else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect");
    let owner = seed_user(&db, "owner").await;
    let member = seed_user(&db, "member").await;
    let outsider = seed_user(&db, "outsider").await;
    let (parent, parent_path) = seed_group(&db, None).await;
    let (child, _) = seed_group(&db, Some((parent, parent_path))).await;
    add_member(&db, parent, member).await;

    // Owner creates a personal task (no group) and a child-group task.
    let personal = insert_task(
        &db,
        owner,
        None,
        "text_batch",
        "queued",
        "test/sql-opt-personal",
    )
    .await;
    let child_task = insert_task(
        &db,
        owner,
        Some(child),
        "text_batch",
        "queued",
        "test/sql-opt-child",
    )
    .await;

    let owner_rows = list_ids(&db, owner, "processing", 50, 0).await;
    assert!(owner_rows.contains(&personal));
    assert!(owner_rows.contains(&child_task));

    let member_rows = list_ids(&db, member, "processing", 50, 0).await;
    assert!(
        member_rows.contains(&child_task),
        "parent member must see a child-group task via descendant access"
    );
    assert!(
        !member_rows.contains(&personal),
        "member must not see another user's personal task"
    );

    let outsider_rows = list_ids(&db, outsider, "processing", 50, 0).await;
    assert!(!outsider_rows.contains(&personal));
    assert!(!outsider_rows.contains(&child_task));

    assert_eq!(
        count(&db, owner, "processing").await,
        owner_rows.len() as i64
    );
    assert_eq!(
        count(&db, member, "processing").await,
        member_rows.len() as i64
    );

    cleanup(
        &db,
        &[personal, child_task],
        &[child, parent],
        &[owner, member, outsider],
    )
    .await;
}

#[tokio::test]
async fn overlapping_membership_never_duplicates_and_filters_paginate() {
    let Some(url) = test_db_url() else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url).await.expect("connect");
    let owner = seed_user(&db, "overlap-owner").await;
    let viewer = seed_user(&db, "overlap-viewer").await;
    let (parent, parent_path) = seed_group(&db, None).await;
    let (child, _) = seed_group(&db, Some((parent, parent_path))).await;
    // Overlapping membership: viewer belongs to both parent and child.
    add_member(&db, parent, viewer).await;
    add_member(&db, child, viewer).await;

    let tag = Uuid::new_v4().to_string();
    let mut ids = Vec::new();
    for i in 0..5 {
        let kind = if i % 2 == 0 {
            "text_batch"
        } else {
            "url_batch"
        };
        ids.push(
            insert_task(
                &db,
                owner,
                Some(child),
                kind,
                "queued",
                &format!("test/sql-opt-{tag}-{i}"),
            )
            .await,
        );
    }

    let rows = db
        .list_tasks(TaskListFilter {
            user_id: viewer,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "processing",
        })
        .await
        .expect("list");
    let viewer_ids: Vec<Uuid> = rows.iter().map(|t| t.id).collect();
    for id in &ids {
        assert!(
            viewer_ids.contains(id),
            "overlapping member must see child task once"
        );
    }
    assert_eq!(
        viewer_ids.iter().filter(|id| ids.contains(id)).count(),
        ids.len(),
        "overlapping membership must not duplicate tasks"
    );
    assert_eq!(
        count(&db, viewer, "processing").await as usize,
        viewer_ids.len(),
        "count must match list length"
    );

    // Kind filter narrows but never widens.
    let text_rows = db
        .list_tasks(TaskListFilter {
            user_id: viewer,
            query: None,
            kind: Some("text_batch"),
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "processing",
        })
        .await
        .expect("filter by kind");
    assert!(!text_rows.is_empty());
    assert!(text_rows.iter().all(|t| t.kind == "text_batch"));
    assert!(text_rows.len() < viewer_ids.len());

    // Pagination partitions without overlap.
    let first = list_ids(&db, viewer, "processing", 2, 0).await;
    let second = list_ids(&db, viewer, "processing", 2, 2).await;
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(first.iter().all(|id| !second.contains(id)));

    // Search filter matches the tagged group path.
    let searched = db
        .list_tasks(TaskListFilter {
            user_id: viewer,
            query: Some(&tag[..8]),
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "processing",
        })
        .await
        .expect("search");
    assert!(
        searched.iter().any(|t| ids.contains(&t.id)),
        "search must find the tagged tasks"
    );

    cleanup(&db, &ids, &[child, parent], &[owner, viewer]).await;
}
