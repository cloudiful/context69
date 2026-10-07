//! SQL contract for the parent projection shared by the task recompute paths.
//!
//! Three statements write `tasks` counters/summary fields from `task_items`:
//! `claim_items.sql`, `recompute.sql`, and `maintain_claim_state.sql`. They are
//! separate files because they run in different transactions, so the invariants
//! that keep them consistent cannot be expressed in the type system and are
//! pinned here instead:
//!
//!   * one current-item ordering (head-of-line) in every path,
//!   * the parent write driven by the items that actually exist,
//!   * the item transition, not the append-only attempt rows, deciding whether
//!     a transition took effect,
//!   * a task kind that the create-time stage map forgets would otherwise be
//!     created already labelled `finalize`.

const CLAIM_ITEMS: &str = include_str!("../../src/sql/db/tasks/claim_items.sql");
const RECOMPUTE: &str = include_str!("../../src/sql/db/tasks/recompute.sql");
const MAINTAIN: &str = include_str!("../../src/sql/db/tasks/maintain_claim_state.sql");
const CREATE: &str = include_str!("../../src/sql/db/tasks/create.sql");
const FINISH_ITEM: &str = include_str!("../../src/sql/db/tasks/finish_item.sql");
const WAIT_ITEM: &str = include_str!("../../src/sql/db/tasks/wait_item.sql");
const PROGRESS_ITEM: &str = include_str!("../../src/sql/db/tasks/progress_item.sql");

/// Strips comments so a contract assertion cannot be satisfied by prose.
fn code(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Slices the body of one CTE out of a statement.
///
/// The item-transition statements end with `<attempt CTE>` followed by a
/// top-level `SELECT`, so the body runs from the CTE's opening parenthesis to
/// that final statement. Scoping the text this way is what makes a per-CTE
/// assertion real: a whole-file match can be satisfied by an unrelated clause.
fn cte_body(sql: &str, name: &str) -> String {
    let code = code(sql);
    let opening = format!("{name} AS (");
    let start = code
        .find(&opening)
        .unwrap_or_else(|| panic!("{name} CTE must exist"))
        + opening.len();
    let body = &code[start..];
    let end = body.find("\nSELECT ").unwrap_or(body.len());
    body[..end].to_string()
}

#[test]
fn every_recompute_path_selects_the_head_of_line_item() {
    // `recompute.sql` and `maintain_claim_state.sql` both pick the current item
    // as the lowest-ordinal non-terminal row. Ranking running above queued
    // above waiting (the old `prio` ordering) let the two paths disagree about
    // which item the parent describes.
    for (name, sql) in [("recompute.sql", RECOMPUTE), ("maintain", MAINTAIN)] {
        let code = code(sql);
        assert!(
            code.contains("status IN ('queued', 'running', 'waiting')"),
            "{name} must restrict the current item to non-terminal rows"
        );
        assert!(
            code.contains("ORDER BY ordinal\n    LIMIT 1")
                || code.contains("ORDER BY ti.ordinal\n        LIMIT 1"),
            "{name} must pick the current item by ordinal alone"
        );
    }
    assert!(
        !code(MAINTAIN).contains("prio"),
        "maintain_claim_state.sql must not rank item statuses ahead of ordinal order"
    );
    // The claim path narrows each parent to its current item the same way.
    assert!(
        code(CLAIM_ITEMS).contains("SELECT DISTINCT ON (item.task_id)")
            && code(CLAIM_ITEMS).contains("ORDER BY item.task_id, item.ordinal"),
        "claim_items.sql must claim the head-of-line item of each parent"
    );
}

#[test]
fn parent_status_follows_the_current_item_not_a_count_priority() {
    // Head-of-line waiting: a parent whose current item is backing off reports
    // `waiting` with that item's reason and retry time even while later
    // siblings sit queued.
    for (name, sql) in [("recompute.sql", RECOMPUTE), ("maintain", MAINTAIN)] {
        let code = code(sql);
        assert!(
            code.contains("ELSE current_item.status"),
            "{name} must derive the parent status from the current item"
        );
        assert!(
            !code.contains("WHEN counts.running_count > 0 THEN 'running'"),
            "{name} must not rank the parent status by running/queued/waiting counts"
        );
    }
}

#[test]
fn the_claim_writes_the_parent_only_for_items_it_claimed() {
    let code = code(CLAIM_ITEMS);
    assert!(
        code.contains("FROM claim_counts counts"),
        "the parent UPDATE must be driven by the rows the claim produced"
    );
    assert!(
        !code.contains("SELECT candidate.id, NULL::uuid AS lease_token"),
        "the lease grant must no longer be issued to admission candidates alone"
    );
    // The projection the claim writes has to be the item-derived one.
    for column in [
        "queued_count = counts.queued_count",
        "running_count = counts.running_count",
        "waiting_count = counts.waiting_count",
    ] {
        assert!(
            code.contains(column),
            "the claim must project {column} in the same transaction"
        );
    }
}

#[test]
fn item_transitions_report_the_item_not_the_attempt_rows() {
    // `task_attempts` is append-only forensics. An attempt that maintenance
    // already interrupted must not make a successful item transition look like
    // a lost lease, because that is what skipped the parent recompute.
    //
    // The item CTE and the attempt CTE are separate concerns and are asserted
    // separately: the attempt close must be gated on the item transition by a
    // predicate *inside the attempt CTE*, not anywhere else in the file. A
    // whole-file match is satisfied vacuously by the statement's own final
    // `SELECT EXISTS (SELECT 1 FROM <item CTE>)`, which is how the missing
    // lease fence in `progress_item.sql` slipped through.
    for (name, sql, item_cte, attempt_cte) in [
        (
            "finish_item.sql",
            FINISH_ITEM,
            "finished",
            "attempt_finished",
        ),
        ("wait_item.sql", WAIT_ITEM, "waiting", "attempt_waited"),
        (
            "progress_item.sql",
            PROGRESS_ITEM,
            "progressed",
            "attempt_progressed",
        ),
    ] {
        let code = code(sql);
        assert!(
            code.contains(&format!(
                "SELECT EXISTS (SELECT 1 FROM {item_cte}) AS \"updated!\""
            )),
            "{name} must report the item transition as `updated`"
        );
        assert!(
            !code.trim_end().ends_with("UPDATE context69.task_attempts"),
            "{name} must not take its row count from the attempt UPDATE"
        );

        let attempt = cte_body(sql, attempt_cte);
        assert!(
            attempt.contains(&format!("AND EXISTS (SELECT 1 FROM {item_cte})")),
            "{name}: {attempt_cte} must close the attempt only when the item \
             transition in {item_cte} actually took effect; a worker whose item \
             lease was rotated must not rewrite its own open attempt"
        );
    }
}

#[test]
fn create_maps_every_task_kind_to_its_entry_stage() {
    let code = code(CREATE);
    for arm in [
        "WHEN 'url_batch' THEN 'download'",
        "WHEN 'file_batch' THEN 'storage'",
        "WHEN 'text_batch' THEN 'storage'",
        "WHEN 'source_sync' THEN 'sync'",
        "WHEN 'delete_batch' THEN 'delete'",
        "WHEN 'translation' THEN 'translation'",
        "WHEN 'vector_rebuild' THEN 'indexing'",
        "WHEN 'git_index' THEN 'indexing'",
    ] {
        assert!(code.contains(arm), "create.sql must map {arm}");
    }
    // `git_index` used to fall through to the `ELSE 'finalize'` arm, so a
    // freshly submitted git-indexing task was created labelled as finished.
    assert_eq!(
        code.matches("ELSE 'finalize'").count(),
        1,
        "only the unknown-kind fallback may map to finalize"
    );
}
