//! SQL-contract tests: the task list/count/items/clear queries must keep the
//! view predicate, user scoping, and active-first ordering that the service
//! layer relies on, the resume statement must stay a same-row transition, and
//! the issue #667 Phase 2B terminal-payload migration must strip only the
//! planned keys under kind/status/file guards.
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

/// Resume is a same-row state transition: it reopens the cancelled task's own
/// items and never inserts a replacement parent task, so the queue keeps one
/// visible record per submission while the item/attempt rows keep their audit
/// history.
#[test]
fn resume_sql_reopens_the_same_items_and_never_inserts_a_task() {
    let sql = include_str!("../../sql/db/tasks/resume_items.sql");
    let code: String = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");

    // The statement is one guarded item UPDATE and nothing else: no parent row
    // is written, so a resume can never create a second visible task.
    assert_eq!(
        code.matches("UPDATE").count(),
        1,
        "resume must be a single item UPDATE"
    );
    assert!(
        code.contains("UPDATE context69.task_items item"),
        "resume must reopen the task's own item rows in place"
    );
    for forbidden in [
        "INSERT INTO",
        "DELETE FROM",
        "context69.task_attempts",
        "UPDATE context69.tasks",
        "library_files",
        "documents",
    ] {
        assert!(
            !code.contains(forbidden),
            "resume must not touch {forbidden}; the item rows are the record"
        );
    }

    // Only unfinished items are reopened, which is what makes a repeated or
    // concurrent resume match nothing and stay idempotent.
    assert!(
        code.contains("item.status IN ('cancelled', 'failed')"),
        "resume must reopen only cancelled and failed items"
    );
    for live in [
        "item.status = 'queued'",
        "item.status = 'running'",
        "item.status = 'waiting'",
    ] {
        assert!(
            !code.contains(live),
            "resume must leave an already-active item alone ({live})"
        );
    }
    for arm in [
        "status = 'queued'",
        "stage = 'processing'",
        "lease_token = NULL",
        // A resumed translation item must re-create its remote jobs.
        "WHEN task.kind = 'translation' THEN item.payload - 'job_ids'",
    ] {
        assert!(
            code.contains(arm),
            "resume must apply the same in-place restart as a retry ({arm})"
        );
    }

    // Authorization is enforced in SQL as well, so an unauthorized caller
    // updates no row instead of relying on the service check alone.
    assert!(
        code.contains("task.user_id = $2"),
        "resume must scope ownership to the calling user"
    );
    assert!(
        code.contains("inherited_groups.role_rank >= 2"),
        "resume must keep the group maintainer rule of retry_items.sql"
    );

    // The per-file guard stays: a file already covered by an active item
    // elsewhere keeps its current processing slot.
    assert!(
        code.contains("active.status IN ('queued', 'running', 'waiting')"),
        "resume must skip files that already have an active item elsewhere"
    );
}

/// The collapsed-row projection and the per-item projection must resolve the
/// same way: a row the user reads and the item behind it must never disagree
/// about which file or which title they are.
#[test]
fn the_file_and_title_projection_is_identical_for_a_task_and_its_items() {
    let statements = [
        ("items.sql", include_str!("../../sql/db/tasks/items.sql")),
        ("get.sql", include_str!("../../sql/db/tasks/get.sql")),
        ("list.sql", include_str!("../../sql/db/tasks/list.sql")),
        (
            "get_internal.sql",
            include_str!("../../sql/db/tasks/get_internal.sql"),
        ),
    ];
    for (name, sql) in statements {
        let code: String = sql
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        for fragment in [
            // File name: library file, else the retained payload filename, else
            // the submitted URL's last path segment.
            "NULLIF(file.filename, '')",
            "NULLIF(item.payload ->> 'filename', '')",
            "WHEN item.payload ? 'url'",
            "^.*/",
            // Title: submitted title, else the title of the document this file
            // produced, else the library file's own stored title. Every candidate
            // is NULLIF-guarded so an empty one falls through.
            "NULLIF(item.payload ->> 'title', '')",
            "NULLIF(doc.title, '')",
            "NULLIF(file.metadata_json ->> 'title', '')",
            // The linked document is reached through the item's own file, never
            // by matching a document on its own keys, and it must belong to the
            // group that owns that file.
            "FROM context69.library_file_documents link",
            "WHERE link.file_id = file.id",
            "AND document.group_id = file.group_id",
            "ORDER BY link.sort_order, link.section_key",
        ] {
            assert!(
                code.contains(fragment),
                "{name} must carry the shared projection fragment `{fragment}`"
            );
        }
        // The document link is per section, so it must stay a single-row
        // LATERAL: a plain join would fan the row out.
        assert!(
            code.contains("SELECT document.title"),
            "{name} must select the linked document through a one-row LATERAL"
        );
        for join in ["LEFT JOIN context69.library_files file ON file.id = item.file_id"] {
            assert!(
                code.contains(join),
                "{name} must keep the 1:1 join `{join}`"
            );
        }
        assert!(
            !code.contains("item.payload -> 'options' -> 'metadata' ->> 'external_id'"),
            "{name} must not reach a document by external id; the file link is the authority"
        );
    }

    // The task queries keep one row per task: the focus item is a LATERAL with
    // LIMIT 1, so the projection can never fan a task row out.
    for (name, sql) in statements.iter().skip(1) {
        assert!(
            sql.contains("LEFT JOIN LATERAL ("),
            "{name} must select the focus item through a LATERAL join"
        );
    }
    for name in ["get.sql", "list.sql", "get_internal.sql"] {
        let sql = statements
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, sql)| *sql)
            .expect("statement");
        // Two bounded selections: the linked document of the item's file and the
        // task's focus item.
        assert_eq!(
            sql.matches("LIMIT 1").count(),
            2,
            "{name} must bound both the linked document and the focus item to one row"
        );
    }
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
