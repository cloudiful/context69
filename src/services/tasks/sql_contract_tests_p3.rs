//! SQL-contract tests for the issue 702 P3 statements.
//!
//! `sql_contract_tests.rs` owns the pre-existing task SQL contracts. This file
//! owns the two statements P3 introduced — the parent/item consistency snapshot
//! and the diagnose item read — and pins the invariants that are easy to break
//! silently: the snapshot must stay read-only, its orphan gauge must ask the
//! same question the P1 reclaim asks, and its item ordering must not disturb
//! the paged items endpoint.

/// Strip comments so a contract assertion tests executable SQL, not prose.
fn sql_code(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn task_consistency_sql_is_read_only_and_scoped() {
    let code = sql_code(include_str!("../../sql/db/tasks/task_consistency.sql"));

    // One statement serves two consumers; a write here would repair state
    // outside the item transitions that own it.
    for forbidden in [
        "UPDATE", "INSERT", "DELETE", "CREATE", "ALTER", "TRUNCATE", "DROP",
    ] {
        assert!(
            !code.contains(forbidden),
            "the consistency snapshot must not {forbidden}: it reports, it never repairs"
        );
    }
    assert!(
        code.contains("($1::uuid IS NULL OR task.id = $1::uuid)"),
        "the scope filter must make NULL mean the whole queue and a uuid mean one task"
    );
    // The per-item attempt/lease forensics are only worth their cost for a
    // single task, so the health scope must skip the branch entirely.
    assert!(
        code.contains("$1::uuid IS NOT NULL"),
        "the item diagnostics branch must be guarded on a single-task scope"
    );

    // The same current-item ordering the transitions use, so diagnose and
    // health cannot disagree with `recompute.sql` about who is current.
    assert!(
        code.contains("WHERE item.status IN ('queued', 'running', 'waiting')")
            && code.contains("ORDER BY item.task_id, item.ordinal"),
        "the current item must be the lowest-ordinal non-terminal item"
    );
    for signal in [
        "running_parent_without_running_item_count",
        "lease_without_running_item_count",
        "open_attempt_count",
        "near_exhaustion_count",
        "oldest_admitted_at",
        "parent_item_mismatch_count",
    ] {
        assert!(
            code.contains(signal),
            "health must carry the {signal} signal"
        );
    }
    assert!(
        code.contains("attempt.finished_at IS NULL"),
        "an open attempt is one without a finish, not a status string"
    );
    // Lease tokens authorize writes and have no diagnostic value.
    assert!(
        !code.contains("lease_token"),
        "the snapshot must project lease deadlines, never lease tokens"
    );
    assert!(
        !code.contains("item.payload") && !code.contains("task.payload"),
        "diagnose explains progress; it never re-exposes item input"
    );
}

#[test]
fn orphan_lease_gauge_matches_the_p1_reclaim_predicate() {
    // Issue 702 P3 review: counting every live parent lease without a live
    // item lease flagged every legitimate backoff/dependency wait. The gauge
    // must ask the same question `maintain_claim_state.sql` asks before it
    // reclaims a slot, so the two never disagree about what an orphan is.
    let code = sql_code(include_str!("../../sql/db/tasks/task_consistency.sql"));
    assert!(
        code.contains("AND (expired_running_item_exists OR NOT claimable_item_exists)"),
        "the orphan gauge must require the same reclaim shape P1 applies"
    );
    assert!(
        code.contains("candidate.attempt_count < 5"),
        "claimability must match the `claim_items.sql` attempt cap, or an \
         exhausted parent would be reported as holding a legitimate slot"
    );
    assert!(
        code.contains("candidate.status IN ('queued', 'running', 'waiting')"),
        "a parent with a claimable item is not an orphan"
    );
    assert!(
        code.contains("lapsed.lease_until <= now()"),
        "the crashed-worker shape needs a lapsed item lease"
    );

    // The reclaim predicate must stay pinned to the maintenance statement, so a
    // change to one cannot silently drift from the other. The two statements
    // alias the same tables differently, so the arms are compared per shape.
    let maintain = sql_code(include_str!("../../sql/db/tasks/maintain_claim_state.sql"));
    for shape in [
        "status IN ('queued', 'running', 'waiting')",
        "attempt_count < 5",
    ] {
        assert!(
            maintain.contains(shape) && code.contains(shape),
            "both statements must keep the reclaim predicate `{shape}`"
        );
    }
    assert!(
        maintain.contains("item.lease_until > now()"),
        "the live-worker predicate must stay pinned to maintenance"
    );
}

#[test]
fn oldest_admitted_age_is_not_measured_from_the_lease_deadline() {
    // Issue 702 P3 review: `min(lease_until)` is a future deadline, so
    // `now - lease_until` clamped every age to zero. Admission time
    // (`started_at`, stamped when the claim admits the parent) is the only
    // timestamp that means "this parent has held a slot since".
    let code = sql_code(include_str!("../../sql/db/tasks/task_consistency.sql"));
    assert!(
        code.contains("SELECT min(started_at) FROM verdict"),
        "oldest_admitted_at must come from admission time"
    );
    assert!(
        !code.contains("min(lease_until)"),
        "a future lease deadline must never be read as an admission time"
    );
}

#[test]
fn the_admission_timestamp_the_age_reads_is_stamped_by_the_admission_claim() {
    // Issue 702 P3 review, second half: pinning `min(started_at)` is only half
    // the claim. If nothing stamps `tasks.started_at` at admission, the gauge
    // is permanently NULL (or, once a submission stamped it, silently means
    // "time since submit" instead of "time holding a slot"). Every projection
    // test seeds `started_at` by hand, so only a cross-statement contract can
    // catch a claim that stopped stamping it.
    let claim = sql_code(include_str!("../../sql/db/tasks/claim_items.sql"));
    assert!(
        claim.contains("started_at = COALESCE(task.started_at, now())"),
        "admission must stamp the parent admission timestamp, and must not \
         overwrite an earlier admission on a later claim"
    );
    assert!(
        claim.contains("started_at = COALESCE(item.started_at, now())"),
        "the item attempt timestamp follows the same first-write rule"
    );
    let create = sql_code(include_str!("../../sql/db/tasks/create.sql"));
    assert!(
        !create.contains("started_at"),
        "a submission must not stamp the admission timestamp: the gauge is \
         defined as age since the parent was admitted, not since it was queued"
    );
}

#[test]
fn consistency_compares_the_item_derived_status() {
    // Issue 702 P3 review: counters alone let a stale terminal/running status
    // read as consistent. The verdict must derive the status from the items the
    // same way `recompute.sql` writes it.
    let code = sql_code(include_str!("../../sql/db/tasks/task_consistency.sql"));
    assert!(
        code.contains("THEN 'status' END"),
        "a diverging parent status must be named as a mismatch"
    );
    assert!(code.contains("fact.parent_status <> fact.derived_status"));
    assert!(
        code.contains("WHEN current.status IS NULL THEN 'queued'"),
        "the derived status must mirror recompute's current-item fallback"
    );
    // The arms must stay in recompute's order, or the derived status would
    // disagree with the column the transitions write.
    let recompute = sql_code(include_str!("../../sql/db/tasks/recompute.sql"));
    for arm in [
        "THEN 'cancelled'",
        "THEN 'succeeded'",
        "THEN 'failed'",
        "THEN 'queued'",
    ] {
        assert!(
            recompute.contains(arm) && code.contains(arm),
            "missing arm {arm}"
        );
    }
    let code_arms = code
        .split("AS derived_status")
        .next()
        .expect("derived status block");
    assert!(
        code_arms.find("THEN 'cancelled'") < code_arms.find("THEN 'succeeded'"),
        "terminal overrides must be evaluated before the current-item fallback"
    );
}

#[test]
fn items_sql_serves_both_orderings_and_the_ordinal_first_page() {
    // Issue 702 P3 review: diagnose promised the lowest ordinals when it
    // truncated, but it paged through the active-first ordering, so a later
    // running/failed item displaced a lower-ordinal one.
    let code = sql_code(include_str!("../../sql/db/tasks/items.sql"));
    assert!(
        code.contains("$5::text = 'ordinal'"),
        "items.sql must expose ordinal-first selection for diagnose"
    );
    assert!(
        code.contains("$6::uuid IS NULL OR item.id = $6::uuid"),
        "items.sql must narrow to one item so a log can resolve its ordinal"
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
            "the paged items endpoint must keep active-first with {arm}"
        );
    }
    assert!(
        code.contains("item.ordinal"),
        "both orderings must tie-break by ordinal"
    );
}
