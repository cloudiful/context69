//! SQL-contract tests: the task list/count/items/clear queries must keep the
//! view predicate, user scoping, and active-first ordering that the service
//! layer relies on.

#[test]
fn task_list_and_count_share_one_view_predicate() {
    let list = include_str!("../../sql/db/tasks/list.sql");
    let count = include_str!("../../sql/db/tasks/count.sql");
    for branch in [
        "task.deleted_at IS NULL AND task.status <> 'succeeded'",
        "task.deleted_at IS NULL AND task.status = 'succeeded'",
        "AND task.deleted_at IS NOT NULL",
    ] {
        assert!(
            list.contains(branch),
            "list.sql must contain view branch {branch}"
        );
        assert!(
            count.contains(branch),
            "count.sql must contain view branch {branch}"
        );
    }
    for view in ["'processing'", "'completed'", "'trash'"] {
        assert!(
            list.contains(view),
            "list.sql must contain view literal {view}"
        );
        assert!(
            count.contains(view),
            "count.sql must contain view literal {view}"
        );
    }
    for sql in [list, count] {
        assert!(
            !sql.contains("::boolean"),
            "legacy trashed boolean branch must be gone"
        );
        assert!(
            !sql.contains("IS NULL AND ("),
            "legacy null-view fallback must be gone"
        );
    }
}

#[test]
fn clear_user_task_history_sql_is_user_scoped_and_view_guarded() {
    let sql = include_str!("../../sql/db/tasks/clear_user_task_history.sql");
    let code: String = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("DELETE FROM context69.tasks"),
        "clear SQL must delete only task rows"
    );
    assert_eq!(
        code.matches("DELETE FROM").count(),
        1,
        "clear SQL must be a single DELETE statement"
    );
    assert!(
        code.contains("user_id = $1"),
        "clear SQL must be scoped to the calling user"
    );
    assert!(
        !code.contains("group_memberships") && !code.contains("inherited_groups"),
        "clear SQL must not widen to group-shared rows; only user_id owns the delete"
    );
    assert!(
        code.contains("$2::text = 'completed'")
            && code.contains("deleted_at IS NULL")
            && code.contains("status = 'succeeded'"),
        "completed branch must match only untrashed succeeded rows"
    );
    assert!(
        code.contains("$2::text = 'trash'")
            && code.contains("deleted_at IS NOT NULL")
            && code.contains("status IN ('succeeded', 'failed', 'cancelled')"),
        "trash branch must match only trashed terminal rows"
    );
    for forbidden in [
        "DELETE FROM context69.task_items",
        "DELETE FROM context69.task_attempts",
        "library_files",
        "documents",
        "qdrant",
        "storage_objects",
    ] {
        assert!(
            !code.contains(forbidden),
            "clear SQL must not directly touch {forbidden}; history cascades, files stay"
        );
    }
}

#[test]
fn task_items_sql_filters_status_and_pins_active_first() {
    let sql = include_str!("../../sql/db/tasks/items.sql");
    let code: String = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("($4::text IS NULL OR item.status = $4::text)"),
        "items.sql must filter by status with a NULL-means-all guard"
    );
    assert!(
        code.contains("WHERE item.task_id = $1"),
        "items.sql must stay scoped to one task"
    );
    for arm in [
        "WHEN 'failed' THEN 0",
        "WHEN 'running' THEN 1",
        "WHEN 'queued' THEN 2",
        "WHEN 'waiting' THEN 3",
        "WHEN 'cancelled' THEN 4",
        "WHEN 'succeeded' THEN 5",
    ] {
        assert!(
            code.contains(arm),
            "items.sql must pin active-first with {arm} and sink succeeded"
        );
    }
    assert!(
        code.contains("item.ordinal"),
        "active-first must tie-break by ordinal for stable cursor paging"
    );
    assert!(
        code.contains("LIMIT $2 OFFSET $3"),
        "cursor paging must stay offset-based (limit $2, offset $3)"
    );
}
