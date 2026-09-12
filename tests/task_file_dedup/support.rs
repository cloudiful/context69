//! Dedup-only fixtures for the issue 332 phase 1 regressions.
//!
//! Kept separate from `tests/task_file_status/support.rs` so the status test
//! crate, which does not use them, compiles without dead-code warnings.

use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

pub async fn insert_cancelled_source_task(
    db: &Database,
    user_id: i64,
    group_id: i64,
    files: &[(Uuid, i32)],
) -> Uuid {
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks \
         (id, user_id, group_id, kind, status, origin, group_path, total_count, queued_count, cancelled_count, stage) \
         VALUES ($1, $2, $3, 'file_batch', 'cancelled', 'manual', 'test/file-status', $4, 0, $4, 'storage')",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .bind(files.len() as i64)
    .execute(db.pool())
    .await
    .expect("insert cancelled source task");
    for (file_id, ordinal) in files {
        sqlx::query(
            "INSERT INTO context69.task_items \
             (id, task_id, ordinal, payload, status, file_id, stage) \
             VALUES ($1, $2, $3, $4, 'cancelled', $5, 'storage')",
        )
        .bind(Uuid::new_v4())
        .bind(task_id)
        .bind(*ordinal)
        .bind(json!({ "file_id": file_id }))
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("insert cancelled source item");
    }
    task_id
}

pub async fn count_active_file_items(db: &Database, file_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_items item \
         JOIN context69.tasks task ON task.id = item.task_id \
         WHERE item.file_id = $1 \
           AND item.status IN ('queued', 'running', 'waiting') \
           AND task.kind = 'file_batch'",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("count active file items")
}
