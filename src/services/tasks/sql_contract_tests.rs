//! SQL-contract tests: the task list/count/items/clear queries must keep the
//! view predicate, user scoping, and active-first ordering that the service
//! layer relies on, and the issue #667 Phase 2B terminal-payload migration
//! must strip only the planned keys under kind/status/file guards.
//!
//! The issue 702 P3 statements have their own module, `sql_contract_tests_p3.rs`.

/// Issue #667 Phase 2B migration, read verbatim so the contract test fails if
/// a guard, kind list, or stripped key drifts from the plan.
const TERMINAL_PAYLOAD_MIGRATION: &str =
    include_str!("../../../migrations/20260930134806_strip_terminal_task_payloads.sql");

fn terminal_payload_migration_code() -> String {
    TERMINAL_PAYLOAD_MIGRATION
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

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

#[test]
fn terminal_payload_migration_strips_only_planned_keys_under_guards() {
    let code = terminal_payload_migration_code();

    // Exactly three payload updates, one per planned retention group, and no
    // other statement kind.
    assert_eq!(
        code.matches("UPDATE ").count(),
        3,
        "the migration must hold exactly three UPDATE statements"
    );
    assert_eq!(
        code.matches("UPDATE context69.task_items").count(),
        3,
        "every update must target task_items, the only payload owner"
    );

    // Succeeded URL items drop the obsolete download artifact; failed and
    // cancelled URL items are never matched.
    assert!(
        code.contains("item.payload - 'download_artifact'"),
        "succeeded URL items must strip the download artifact"
    );
    assert!(
        code.contains("item.status = 'succeeded'"),
        "the artifact strip must be guarded to succeeded items"
    );
    assert!(
        code.contains("task.kind = 'url_batch'"),
        "the artifact strip must be guarded to URL tasks"
    );

    // Terminal text/file items drop the reconstructible section checkpoint.
    assert!(
        code.contains("item.payload - 'section_payload' - 'indexing_checkpoint'"),
        "terminal text/file items must strip section_payload and indexing_checkpoint"
    );
    assert!(
        code.contains("item.status IN ('succeeded', 'failed', 'cancelled')"),
        "the checkpoint strip must cover exactly the terminal statuses"
    );
    assert!(
        code.contains("task.kind IN ('text_batch', 'file_batch')"),
        "the checkpoint strip must be scoped to the text/file ingestion kinds"
    );
    assert!(
        code.contains("item.payload ? 'section_payload'")
            && code.contains("item.payload ? 'indexing_checkpoint'"),
        "the checkpoint strip must match only rows that still carry a key"
    );

    // Succeeded text items with a durably succeeded file drop the inline body.
    assert!(
        code.contains("item.payload - 'content'"),
        "succeeded text items must strip the inline content"
    );
    assert!(
        code.contains("item.payload ? 'content'"),
        "the content strip must match only rows that still carry content"
    );
    assert!(
        code.contains("task.kind = 'text_batch'"),
        "the content strip must be guarded to text tasks"
    );
    assert!(
        code.contains("file.ingest_status = 'succeeded'"),
        "the content strip must require a durably succeeded library file"
    );

    // Every update requires a durable file reference, so rows without one are
    // left for a future retry/rerun.
    assert_eq!(
        code.matches("item.file_id IS NOT NULL").count(),
        3,
        "all three updates must require a durable file_id"
    );

    // No row or schema mutation, and no table other than the task payload is
    // written.
    for forbidden in [
        "DELETE",
        "INSERT INTO",
        "DROP ",
        "ALTER ",
        "TRUNCATE",
        "CREATE ",
        "UPDATE context69.tasks",
        "UPDATE context69.library_files",
        "UPDATE context69.library_storage_objects",
        "SET status",
    ] {
        assert!(
            !code.contains(forbidden),
            "terminal-payload retention must not {forbidden}"
        );
    }

    // Live items are never stripped: only terminal statuses appear.
    for live in ["'running'", "'queued'", "'waiting'"] {
        assert!(
            !code.contains(live),
            "the migration must not touch live item status {live}"
        );
    }
}
