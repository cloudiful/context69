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
        code.contains("(candidate.attempt_count - candidate.paused_attempts) < 5"),
        "claimability must match the `claim_candidates.sql` attempt cap, or an \
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
    for source in [&maintain, &code] {
        assert!(
            source.contains("status IN ('queued', 'running', 'waiting')"),
            "both statements must keep the reclaim status predicate"
        );
    }
    assert!(
        maintain.contains("(item.attempt_count - item.paused_attempts) < 5"),
        "maintenance must use the effective attempt budget"
    );
    assert!(
        code.contains("(candidate.attempt_count - candidate.paused_attempts) < 5"),
        "the consistency gauge must use the effective attempt budget"
    );
    assert!(
        maintain.contains("attempt.fencing_token = item.lease_token")
            && maintain.contains("attempt.lease_expires_at > clock_timestamp()"),
        "the live-worker predicate must stay pinned to the library attempt authority"
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
    let claim = sql_code(include_str!("../../sql/db/tasks/mirror_claim.sql"));
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

#[test]
fn a_runnable_rebuild_gates_admission_to_one_oldest_rebuild() {
    // Issue 51 P1: the rebuild's priority is admission order, not a rewritten
    // creation time. While a due rebuild exists it either drains running
    // ordinary attempts (admitting nothing) or admits only the one selected
    // rebuild; the ordinary policy stays the existing one.
    let code = sql_code(include_str!("../../sql/db/tasks/claim_candidates.sql"));
    assert!(code.contains("runnable_rebuild AS"));
    assert!(
        code.contains("item.kind = 'vector_rebuild'")
            && code.contains("item.execution_task_id IS NOT NULL"),
        "a runnable rebuild needs the rebuild kind and a captured execution link"
    );
    assert!(
        code.contains("ORDER BY item.task_created_at, item.task_id") && code.contains("LIMIT 1"),
        "exactly one oldest rebuild is selected"
    );
    assert!(
        code.contains("THEN 'drain'") && code.contains("THEN 'rebuild'"),
        "a due rebuild drains running ordinary attempts and then owns admission"
    );
    assert!(
        code.contains("EXISTS (SELECT 1 FROM live_running WHERE kind <> 'vector_rebuild')"),
        "ordinary running is a live attempt, not a task status"
    );
    assert!(
        code.contains("WHERE kind = 'vector_rebuild'"),
        "a running rebuild is detected from live running attempts"
    );
    assert!(
        code.matches("gate.mode = 'normal'").count() >= 3,
        "the gate must apply to work, admitted_work and chosen"
    );
    for predicate in [
        "(ti.attempt_count - ti.paused_attempts) < 5",
        "parent.pause_state = 'unpaused'",
        "ti.status = 'waiting'",
        "COALESCE(ti.waiting_reason, '') = 'docling'",
        "ti.dependency_key = 'vector_rebuild_paused'",
    ] {
        assert!(
            code.contains(predicate),
            "the existing eligibility predicate must survive: {predicate}"
        );
    }
}

#[test]
fn idle_ordinary_lease_release_is_narrow_and_uses_the_same_gate() {
    let claim = sql_code(include_str!("../../sql/db/tasks/claim_candidates.sql"));
    let release = sql_code(include_str!(
        "../../sql/db/tasks/release_gated_ordinary_leases.sql"
    ));

    // Only the admission authority of an idle parent is cleared.
    assert!(
        release.contains("SET lease_token = NULL") && release.contains("lease_until = NULL"),
        "the release must clear the slot, never an attempt"
    );
    assert!(
        release.contains("task.kind <> 'vector_rebuild'")
            && release.contains("task.kind = 'vector_rebuild'")
            && release.contains("task.status IN ('queued', 'running', 'waiting')")
            && release.contains("task.deleted_at IS NULL"),
        "the release is limited to active parents in both scopes"
    );
    assert!(
        release.contains("item.status = 'running'")
            && release.contains("attempt.fencing_token = item.lease_token")
            && release.contains("attempt.lease_expires_at > clock_timestamp()"),
        "a parent with a live library attempt is never released"
    );
    for owned in ["task_items item", "task_attempts"] {
        assert!(
            !release.contains(&format!("UPDATE context69.{owned}")),
            "the release must not mutate {owned}"
        );
    }

    // The gate that justifies releasing capacity is the rebuild gate, with the
    // same eligibility predicate the claim statement uses.
    assert!(release.contains("runnable_rebuild AS"));
    for predicate in [
        "(item.attempt_count - item.paused_attempts) < 5",
        "item.execution_task_id IS NOT NULL",
        "item.pause_state = 'unpaused'",
        "item.task_status IN ('queued', 'running', 'waiting')",
        "COALESCE(item.waiting_reason, '') = 'docling'",
        "item.dependency_key = 'vector_rebuild_paused'",
    ] {
        assert!(
            release.contains(predicate),
            "the release gate must match the claim eligibility: {predicate}"
        );
    }
    for predicate in [
        "(ti.attempt_count - ti.paused_attempts) < 5",
        "COALESCE(ti.waiting_reason, '') = 'docling'",
        "ti.dependency_key = 'vector_rebuild_paused'",
    ] {
        assert!(
            claim.contains(predicate),
            "the claim eligibility must stay pinned to the release gate: {predicate}"
        );
    }
}

#[test]
fn a_parked_rebuild_releases_its_slot_but_the_selected_and_live_ones_do_not() {
    // Issue 51 P2 R1: a rebuild that parks for pause, dependency, retry backoff,
    // a missing execution link or an exhausted budget is not runnable, so its
    // idle parent lease must not hold capacity until the TTL. The selected
    // runnable rebuild and every live attempt stay protected.
    let claim = sql_code(include_str!("../../sql/db/tasks/claim_candidates.sql"));
    let release = sql_code(include_str!(
        "../../sql/db/tasks/release_gated_ordinary_leases.sql"
    ));

    assert!(
        release.contains("release_scope AS")
            && release.contains(
                "task.kind = 'vector_rebuild'\n              AND task.id IS DISTINCT FROM gate.selected_rebuild_task_id"
            ),
        "an unselected rebuild parent must release its idle slot"
    );
    assert!(
        release.contains("(SELECT task_id FROM runnable_rebuild) AS selected_rebuild_task_id"),
        "the selected runnable rebuild must be named from the runnable selection"
    );
    assert!(
        release.contains("task.kind <> 'vector_rebuild' AND gate.rebuild_gates"),
        "ordinary idle leases must still be released only while a rebuild gates admission"
    );
    assert!(
        !release.contains("task.kind = 'vector_rebuild' AND gate.rebuild_gates"),
        "the parked rebuild scope must not depend on a runnable/live rebuild gate"
    );

    // The release must keep exactly the claim's oldest selection, and its
    // live-attempt fence must remain the same nonterminal attempt predicate.
    for source in [&claim, &release] {
        assert!(
            source.contains("ORDER BY item.task_created_at, item.task_id")
                && source.contains("LIMIT 1"),
            "the oldest runnable rebuild selection must stay identical"
        );
    }
    assert!(
        release.contains("item.status = 'running'")
            && release.contains("attempt.fencing_token = item.lease_token")
            && release.contains("attempt.lease_expires_at > clock_timestamp()"),
        "a live library attempt must keep its parent slot"
    );

    // Slot bookkeeping only: the release never changes item/attempt/task state.
    for forbidden in [
        "UPDATE context69.task_items",
        "UPDATE context69.task_attempts",
        "SET status",
        "SET pause_state",
        "SET finished_at",
    ] {
        assert!(
            !release.contains(forbidden),
            "the release must not perform {forbidden}"
        );
    }
}

#[test]
fn the_idle_release_runs_inside_the_admission_lock_before_selection() {
    // The release frees capacity, so it must observe the same serialized
    // snapshot as admission and precede the candidate statement. Both root
    // entrypoints must run it.
    let claims = include_str!("../../db/tasks/claims.rs");
    let lock = "lock_task_admission(&mut tx).await?;";
    let release = "release_gated_ordinary_leases(&mut tx).await?;";
    let select = "claim_candidates(&mut tx, capacity).await?;";

    assert_eq!(
        claims.matches(release).count(),
        2,
        "both claim entrypoints must release idle ordinary leases"
    );
    let mut cursor = 0;
    for _ in 0..2 {
        let lock_at = claims[cursor..]
            .find(lock)
            .map(|offset| cursor + offset)
            .expect("admission lock");
        let release_at = claims[lock_at..]
            .find(release)
            .map(|offset| lock_at + offset)
            .expect("release after the admission lock");
        let select_at = claims[release_at..]
            .find(select)
            .map(|offset| release_at + offset)
            .expect("selection after the release");
        assert!(lock_at < release_at && release_at < select_at);
        cursor = select_at + select.len();
    }
}

#[test]
fn the_lease_mirror_is_nonblocking_monotonic_and_owner_fenced() {
    // Issue 55 P1: a page transaction may hold the item row for a long install.
    // The mirror must skip that lock instead of waiting for it, never move the
    // authoritative deadline backwards, and only refresh a still-live returned
    // deadline.
    let code = sql_code(include_str!(
        "../../sql/db/tasks/heartbeat_item_skip_locked.sql"
    ));
    assert!(
        code.contains("FOR UPDATE SKIP LOCKED"),
        "a busy install page must be skipped, not waited for"
    );
    assert!(
        code.contains("GREATEST(COALESCE(item.lease_until, $3), $3)"),
        "the mirror must keep the authoritative deadline monotonic"
    );
    assert!(
        code.contains("AND $3 > clock_timestamp()"),
        "only a still-live authoritative deadline may refresh the mirror"
    );
    for predicate in [
        "item.id = $1",
        "item.lease_token = $2",
        "item.status = 'running'",
        "item.execution_task_id = $4",
        "item.execution_generation = $5",
    ] {
        assert!(
            code.contains(predicate),
            "the mirror must keep the exact owner predicate: {predicate}"
        );
    }
    assert!(
        !code.contains("lease_until > clock_timestamp()"),
        "an expired mirror may be refreshed by its owner's renewed deadline"
    );
}

#[test]
fn the_ownership_probe_takes_no_lock_and_never_writes() {
    // A skipped mirror update is not a lost lease. The probe reads the last
    // committed grant without locking the row the install page holds.
    let code = sql_code(include_str!("../../sql/db/tasks/probe_item_lease.sql"));
    for forbidden in ["UPDATE", "INSERT", "DELETE", "FOR UPDATE", "FOR SHARE"] {
        assert!(
            !code.contains(forbidden),
            "the ownership probe must not {forbidden}"
        );
    }
    for predicate in [
        "id = $1",
        "lease_token = $2",
        "status = 'running'",
        "execution_task_id = $3",
        "execution_generation = $4",
    ] {
        assert!(
            code.contains(predicate),
            "the probe must keep the exact owner predicate: {predicate}"
        );
    }
}

#[test]
fn scheduler_live_execution_is_the_live_library_attempt() {
    // P2 R1 finding 2: the item mirror can expire while a long install page
    // still owns a renewed library attempt. Every scheduler/accounting path
    // that decides whether a worker is live must read the attempt authority,
    // tied to the exact item grant, instead of the mirror deadline.
    let sources = [
        include_str!("../../sql/db/tasks/claim_candidates.sql"),
        include_str!("../../sql/db/tasks/maintain_claim_state.sql"),
        include_str!("../../sql/db/tasks/release_gated_ordinary_leases.sql"),
        include_str!("../../sql/db/tasks/recover_expired_item.sql"),
        include_str!("../../sql/db/tasks/pause/recover_expired_paused_item.sql"),
    ];
    for source in sources {
        let code = sql_code(source);
        for predicate in [
            "cloudiful_tasks.attempts",
            "fencing_token",
            "lease_token",
            "attempt.lease_expires_at > clock_timestamp()",
            "library_task.namespace = 'context69'",
            "library_task.queue = 'items'",
            "library_task.kind = 'context69.item'",
        ] {
            assert!(
                code.contains(predicate),
                "live execution must read the attempt authority: {predicate}"
            );
        }
        assert!(
            !code.contains("item.lease_until >")
                && !code.contains("other.lease_until >")
                && !code.contains("item.lease_until <")
                && !code.contains("other.lease_until <"),
            "the scheduler must not use a stale item mirror as live authority"
        );
    }

    // The same predicate keeps the item status and the generation link, so an
    // expired attempt cannot protect a ghost task and another producer's
    // attempt cannot protect a context69 item.
    let claim = sql_code(sources[0]);
    assert!(claim.contains("item.status = 'running'"));
    assert!(claim.contains("attempt.task_id = item.execution_task_id"));
    assert!(claim.contains("attempt.fencing_token = item.lease_token"));
}

#[test]
fn cutover_quiescence_keeps_the_domain_mirror_and_verifies_every_link() {
    // C4: the cutover statement is domain-only. The library half is evaluated
    // by the queue capability on the caller's transaction, so the statement
    // must not regress into reading the task tables directly.
    let cutover = sql_code(include_str!(
        "../../sql/db/tasks/cutover/live_execution_leases.sql"
    ));
    for predicate in [
        "FROM context69.task_items",
        "status = 'running'",
        "lease_until > clock_timestamp()",
    ] {
        assert!(
            cutover.contains(predicate),
            "the cutover domain mirror must stay intact: {predicate}"
        );
    }
    for forbidden in ["cloudiful_tasks", "attempt", "task_attempts"] {
        assert!(
            !cutover.contains(forbidden),
            "no task-table SQL may remain in the cutover statement: {forbidden}"
        );
    }

    // Every distinct linked identifier is selected, regardless of item status,
    // and globally sorted so chunked verification is complete and stable.
    let linked = sql_code(include_str!(
        "../../sql/db/tasks/cutover/linked_execution_task_ids.sql"
    ));
    for predicate in [
        "SELECT DISTINCT execution_task_id",
        "WHERE execution_task_id IS NOT NULL",
        "ORDER BY execution_task_id",
    ] {
        assert!(
            linked.contains(predicate),
            "the linked-ID selector must stay complete and ordered: {predicate}"
        );
    }
    for status_filter in ["status", "stage", "retired"] {
        assert!(
            !linked.contains(status_filter),
            "the linked-ID selector must not filter candidates by {status_filter}"
        );
    }

    // The helper verifies every batch through the library capability on the
    // same caller transaction and never short-circuits after a live batch.
    let helper = sql_code(include_str!("../../db/tasks/cutover.rs"));
    assert!(helper.contains("has_live_attempts_in(&mut *tx, &task_ids)"));
    assert!(helper.contains("chunks(cloudiful_tasks::MAX_TASK_BATCH_SIZE)"));
    assert!(helper.contains("library_live = true;"));
    assert!(
        !helper.contains("break"),
        "a later missing or foreign linkage must still fail closed"
    );
}

#[test]
fn page_start_uses_the_trusted_renewal_deadline_within_the_fenced_write() {
    // P2 R1 finding 1: a busy background mirror is not a fresh page fence. The
    // page must renew authority first, then open its fenced write with that
    // exact returned deadline, which the row lock accepts only while it is
    // live and mirrors before page work. The ordinary lock rule stays for
    // every caller that passes no override.
    let lock = sql_code(include_str!("../../sql/db/tasks/lock_task_item.sql"));
    assert!(lock.contains("$5::timestamptz IS NOT NULL"));
    assert!(lock.contains("$5 > clock_timestamp()"));
    assert!(lock.contains("$5::timestamptz IS NULL"));
    assert!(lock.contains("item.lease_until > clock_timestamp()"));

    let fence = sql_code(include_str!("../../library_store/task_write_fence.rs"));
    assert!(fence.contains("begin_task_write_with_lease"));
    assert!(fence.contains("proof.binds("));
    assert!(fence.contains("Some(authoritative_until)"));
    assert!(fence.contains("heartbeat_item_skip_locked.sql"));

    let page = sql_code(include_str!("../../services/sync/execution.rs"));
    assert!(page.contains("renew_task_item_lease"));
    assert!(page.contains("begin_task_write_with_lease"));
    assert!(
        page.contains(
            "install_completion(barrier.installed_chunks, barrier.install_total_chunks, true)?"
        ) && page.contains("if !write.commit().await?"),
        "the empty final page must commit its fresh mirror instead of rolling it back"
    );
    assert!(
        !page.contains("CLAIM_LOST_MESSAGE"),
        "the claim loss must not travel as a generic message"
    );
}
