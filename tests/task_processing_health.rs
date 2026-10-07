//! Processing-health parent-consistency contract regressions (issue 702 P3).
//!
//! `/healthz` already reported the item-level queue shape; issue 702 P3 adds the
//! optional parent-level gauges that say whether the orchestration record still
//! agrees with the execution record. These tests pin the wire shape of that
//! additive field: it is omitted when unmeasured, it survives a round trip, and
//! the queue snapshot query it hangs off stays item-scoped.

use context69_contracts::{
    HealthResponse, HealthStatus, LibraryDependencyGateResponse, LibraryProcessingMetric,
    LibraryProcessingQueueHealth, TaskQueueConsistencyHealth,
};

fn queue_health() -> LibraryProcessingQueueHealth {
    LibraryProcessingQueueHealth {
        pending_count: 4,
        queued_count: 2,
        oldest_pending_age_seconds: None,
        oldest_queued_age_seconds: None,
        oldest_waiting_age_seconds: None,
        recent_failure_count: 1,
        docling_dependency_waiting_count: 1,
        stale_waiting_count: 0,
        status_counts: vec![LibraryProcessingMetric {
            key: "waiting".to_string(),
            count: 2,
        }],
        stage_counts: Vec::new(),
        waiting_reason_counts: vec![LibraryProcessingMetric {
            key: "dependency".to_string(),
            count: 1,
        }],
        dependency_counts: vec![LibraryProcessingMetric {
            key: "docling".to_string(),
            count: 1,
        }],
        processed_last_hour: 60,
        failed_last_hour: 6,
        processing_rate_per_minute: 1.0,
        failure_rate_percent: 10.0,
        parent_consistency: None,
    }
}

#[test]
fn queue_health_omits_parent_consistency_until_it_is_measured() {
    // The field is additive: a deployment that never filled it must not emit a
    // misleading empty gauge, and the item-level payload stays byte-identical.
    let queue = queue_health();
    let value = serde_json::to_value(&queue).expect("serialize queue health");
    assert!(
        !value
            .as_object()
            .expect("object")
            .contains_key("parent_consistency"),
        "an unmeasured gauge is omitted, not null: {value}"
    );
    assert_eq!(
        value["processed_last_hour"], 60,
        "the item-level gauges must be untouched by the additive field"
    );
}

#[test]
fn parent_consistency_gauges_serialize_additively_with_every_signal() {
    let mut queue = queue_health();
    queue.parent_consistency = Some(TaskQueueConsistencyHealth {
        parent_count: 3,
        active_parent_count: 1,
        dependency_waiting_parent_count: 1,
        parent_status_counts: vec![LibraryProcessingMetric {
            key: "waiting".to_string(),
            count: 1,
        }],
        running_parent_without_running_item_count: 2,
        lease_without_running_item_count: 3,
        open_attempt_count: 4,
        near_exhaustion_item_count: 5,
        oldest_admitted_age_seconds: Some(42),
        parent_item_mismatch_count: 6,
    });
    let gauges = serde_json::to_value(&queue).expect("serialize")["parent_consistency"].clone();
    for (field, expected) in [
        ("parent_count", 3),
        ("active_parent_count", 1),
        // The dependency-gate signal lives with the parent gauges so an
        // operator sees parked parents without cross-referencing the gate list.
        ("dependency_waiting_parent_count", 1),
        ("running_parent_without_running_item_count", 2),
        ("lease_without_running_item_count", 3),
        ("open_attempt_count", 4),
        ("near_exhaustion_item_count", 5),
        ("oldest_admitted_age_seconds", 42),
        ("parent_item_mismatch_count", 6),
    ] {
        assert_eq!(gauges[field], expected, "gauge {field} must serialize");
    }
    assert_eq!(gauges["parent_status_counts"][0]["key"], "waiting");

    // Defaults are the "nothing is wrong" reading, which is also what a caller
    // gets for a task-free queue.
    let empty = serde_json::to_value(TaskQueueConsistencyHealth::default()).expect("defaults");
    assert_eq!(empty["parent_item_mismatch_count"], 0);
    assert_eq!(empty["lease_without_running_item_count"], 0);
    assert!(
        empty["oldest_admitted_age_seconds"].is_null(),
        "an unmeasured age is absent, never zero"
    );
}

#[test]
fn health_response_round_trips_the_parent_consistency_gauges() {
    let mut queue = queue_health();
    queue.parent_consistency = Some(TaskQueueConsistencyHealth {
        parent_item_mismatch_count: 2,
        lease_without_running_item_count: 1,
        ..TaskQueueConsistencyHealth::default()
    });
    let response = HealthResponse {
        status: HealthStatus::Ok,
        indexed_chunks: Some(1),
        db_ok: None,
        qdrant_ok: None,
        library_processing_ready: Some(true),
        library_dependency_gates: Some(vec![LibraryDependencyGateResponse {
            dependency_key: "s3".to_string(),
            state: "closed".to_string(),
            failure_count: 0,
            next_probe_at: None,
            last_error: None,
            last_transition_at: chrono::Utc::now(),
            last_success_at: None,
        }]),
        library_processing_queue: Some(queue),
    };
    let decoded: HealthResponse =
        serde_json::from_value(serde_json::to_value(&response).expect("serialize health"))
            .expect("decode health");
    let gauges = decoded
        .library_processing_queue
        .expect("queue health")
        .parent_consistency
        .expect("parent gauges");
    assert_eq!(gauges.parent_item_mismatch_count, 2);
    assert_eq!(gauges.lease_without_running_item_count, 1);
    assert_eq!(gauges.active_parent_count, 0);
}

#[test]
fn orphan_and_admission_gauges_read_the_correct_columns() {
    // Issue 702 P3 review, on the projection the service reads:
    //  * `oldest_admitted_at` must be an admission timestamp, so the age is a
    //    positive number of seconds rather than a clamp of `now - lease_until`;
    //  * the orphan gauge must be the P1 reclaim shape, so a legitimate
    //    waiting/backoff parent holding its slot is never counted.
    let consistency = include_str!("../src/sql/db/tasks/task_consistency.sql");
    assert!(
        consistency.contains("SELECT min(started_at) FROM verdict"),
        "the admitted age must be measured from admission time"
    );
    assert!(
        !consistency.contains("min(lease_until)"),
        "a future lease deadline must never be read as an admission time"
    );
    assert!(
        consistency.contains("AND (expired_running_item_exists OR NOT claimable_item_exists)"),
        "the orphan gauge must require the reclaim shape P1 applies"
    );

    // The gauge is an age in seconds, so a measured admission must serialize as
    // a positive count rather than being dropped or clamped.
    let measured = TaskQueueConsistencyHealth {
        oldest_admitted_age_seconds: Some(1_200),
        ..TaskQueueConsistencyHealth::default()
    };
    assert_eq!(
        serde_json::to_value(&measured).expect("serialize")["oldest_admitted_age_seconds"],
        1_200
    );
}

#[test]
fn every_health_gauge_maps_to_a_column_the_consistency_statement_projects() {
    // Issue 702 P3 review: `parent_consistency_health` is a hand-written
    // projection from the statement's row to the wire struct, so a field can
    // be renamed, dropped, or read from the wrong column without any compiler
    // help. This pins the mapping in both directions: every gauge names a
    // column the statement actually selects, and every parent-level column the
    // statement selects is reported.
    let consistency = include_str!("../src/sql/db/tasks/task_consistency.sql");
    let projection = include_str!("../src/services/library/processing_health.rs");

    // Columns the health projection is allowed to read, and the gauge each one
    // feeds. Keyed on the struct literal so a renamed field fails here.
    let mapping = [
        ("parent_count", "parent_count"),
        ("active_parent_count", "active_parent_count"),
        (
            "dependency_waiting_parent_count",
            "dependency_waiting_parent_count",
        ),
        ("parent_status_counts", "parent_status_counts"),
        (
            "running_parent_without_running_item_count",
            "running_parent_without_running_item_count",
        ),
        (
            "lease_without_running_item_count",
            "lease_without_running_item_count",
        ),
        ("open_attempt_count", "open_attempt_count"),
        ("near_exhaustion_item_count", "near_exhaustion_item_count"),
        ("oldest_admitted_age_seconds", "oldest_admitted_at"),
        ("parent_item_mismatch_count", "parent_item_mismatch_count"),
    ];
    let projection_body: String = projection
        .split("fn parent_consistency_health")
        .nth(1)
        .expect("the health projection exists")
        .split("\n}")
        .next()
        .expect("the projection body")
        .to_string();
    for (gauge, column) in mapping {
        assert!(
            consistency.contains(column),
            "the consistency statement must project `{column}` for gauge `{gauge}`"
        );
        assert!(
            projection_body.contains(&format!("{gauge}:")),
            "the health projection must set gauge `{gauge}`"
        );
        assert!(
            projection_body.contains(&format!("consistency.{column}")),
            "gauge `{gauge}` must read column `{column}`"
        );
    }
    // The two single-task columns are documented as meaningless for the
    // whole-queue scope, so the health projection must not read them.
    for single_task_only in ["current_item_id", "scoped_lease_until", "item_diagnostics"] {
        assert!(
            !projection_body.contains(&format!("consistency.{single_task_only}")),
            "`{single_task_only}` has no meaning in the whole-queue health scope"
        );
    }
    // And the admitted age must be the one computed from the admission
    // timestamp, not from any lease column.
    assert!(
        projection_body.contains("queue_age_seconds(consistency.oldest_admitted_at, now)"),
        "the admitted age must come from the admission timestamp"
    );
    // `lease_without_running_item_count` is a gauge name, so the prohibition is
    // on reading a lease *column*: a future deadline is not an admission time.
    assert!(
        !projection_body.contains("consistency.lease_until"),
        "the parent-level health projection must not read a lease column"
    );
    assert!(
        !projection_body.contains("queue_age_seconds(consistency.lease"),
        "no age may be computed from a lease deadline"
    );
}

#[test]
fn near_exhaustion_fires_before_the_attempt_cap_the_claim_enforces() {
    // The gauge is only actionable while a claim can still succeed:
    // `claim_items.sql` admits `attempt_count < 5`, so the last claimable
    // attempt must already be inside the near-exhaustion band, and the cap
    // itself must not be reported there (an at-cap item is the orphan signal's
    // business, not a warning an operator can act on).
    let consistency = include_str!("../src/sql/db/tasks/task_consistency.sql");
    assert!(
        consistency.contains("item.attempt_count >= 4"),
        "near exhaustion must start one attempt before the cap"
    );
    assert!(
        !consistency.contains("item.attempt_count >= 5"),
        "the near-exhaustion band must not start at the cap itself"
    );
    // Both predicates then use the same threshold, so the band and the
    // claimability test cannot drift apart.
    assert!(consistency.contains("candidate.attempt_count < 5"));
    assert!(consistency.contains("near_exhaustion_count"));
}

#[test]
fn processing_health_snapshot_stays_item_scoped() {
    let sql = include_str!("../src/sql/db/tasks/processing_health.sql");
    assert!(
        !sql.contains("parent_consistency"),
        "the queue snapshot must not grow a parent projection field"
    );
    assert!(
        !sql.contains("context69.tasks"),
        "the queue snapshot stays item-scoped"
    );
    let consistency = include_str!("../src/sql/db/tasks/task_consistency.sql");
    assert!(
        consistency.contains("$1::uuid IS NULL OR task.id = $1::uuid"),
        "one statement must serve both the whole-queue gauges and one task"
    );
    assert!(
        !consistency.contains("UPDATE")
            && !consistency.contains("INSERT")
            && !consistency.contains("DELETE"),
        "the consistency snapshot is read-only: repair stays in the transitions"
    );
}
