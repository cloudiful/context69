//! The task diagnose projection (issue 702 P3).
//!
//! `GET /v1/tasks/{task_id}/diagnose` answers from three separate reads — the
//! parent row, its items, and the consistency statement — so this module owns
//! the join between them: the per-item lease deadlines and attempt forensics the
//! consistency statement returns are keyed back onto the item rows, and the
//! whole thing becomes one response. `responses.rs` keeps the ordinary task and
//! item projections this one is built beside.

use std::collections::HashMap;

use anyhow::Result;
use chrono::Utc;
use context69_contracts::{
    LibraryDependencyGateResponse, TaskAttemptView, TaskConsistencyReport, TaskDiagnoseItem,
    TaskDiagnoseParent, TaskDiagnoseResponse, TaskItemStatus, TaskKind, TaskStatus,
};
use uuid::Uuid;

use context69_contracts::TaskProgress;

use super::responses::{parse_item_status, parse_kind, parse_status};
use crate::db::{
    StoredTask, StoredTaskAttempt, StoredTaskItem, TaskConsistencyRow, TaskItemDiagnostic,
};

/// Upper bound on the items one diagnose response carries. Diagnose is an
/// operator inspection endpoint, not an export: a task with more items than
/// this reports `items_truncated` and returns its lowest ordinals.
pub(super) const DIAGNOSE_ITEM_LIMIT: i64 = 500;

/// Build the diagnose projection for one task.
///
/// The parent row, its items, and the consistency verdict come from separate
/// reads, so this joins them here: item leases and attempt forensics live in
/// the consistency statement (the one place that reads `task_attempts`) and
/// are keyed back onto the item rows by id. Items missing from the diagnostics
/// array still project, without lease or attempt detail, rather than vanishing.
pub(super) fn task_diagnose_response(
    task: StoredTask,
    items: Vec<StoredTaskItem>,
    items_truncated: bool,
    consistency: TaskConsistencyRow,
    dependency_gates: Vec<LibraryDependencyGateResponse>,
) -> Result<TaskDiagnoseResponse> {
    let diagnostics: HashMap<Uuid, TaskItemDiagnostic> =
        serde_json::from_value(consistency.item_diagnostics.clone())
            .map(|entries: Vec<TaskItemDiagnostic>| {
                entries
                    .into_iter()
                    .map(|entry| (entry.item_id, entry))
                    .collect()
            })
            .map_err(|error| {
                anyhow::anyhow!("task consistency item diagnostics are unreadable: {error}")
            })?;
    let items = items
        .into_iter()
        .map(|item| {
            let diagnostic = diagnostics.get(&item.id);
            TaskDiagnoseItem {
                item_id: item.id,
                ordinal: item.ordinal,
                status: parse_item_status(&item.status).unwrap_or(TaskItemStatus::Failed),
                stage: item.stage,
                waiting_reason: item.waiting_reason,
                dependency_key: item.dependency_key,
                next_attempt_at: item.next_attempt_at,
                failure_stage: item.failure_stage,
                error_message: item.error_message,
                attempt_count: item.attempt_count,
                retryable: item.retryable,
                lease_expires_at: diagnostic.and_then(|entry| entry.lease_expires_at),
                active_attempt: diagnostic
                    .and_then(|entry| entry.active_attempt.as_ref())
                    .map(task_attempt_view),
                latest_attempt: diagnostic
                    .and_then(|entry| entry.latest_attempt.as_ref())
                    .map(task_attempt_view),
                created_at: item.created_at,
                started_at: item.started_at,
                finished_at: item.finished_at,
            }
        })
        .collect();
    let mismatches: Vec<String> = serde_json::from_value(consistency.mismatch_fields.clone())
        .map_err(|error| anyhow::anyhow!("task consistency mismatches are unreadable: {error}"))?;
    Ok(TaskDiagnoseResponse {
        task: TaskDiagnoseParent {
            task_id: task.id,
            kind: parse_kind(&task.kind).unwrap_or(TaskKind::TextBatch),
            status: parse_status(&task.status).unwrap_or(TaskStatus::Failed),
            progress: TaskProgress {
                total: task.total_count,
                queued: task.queued_count,
                running: task.running_count,
                waiting: task.waiting_count,
                succeeded: task.succeeded_count,
                failed: task.failed_count,
                cancelled: task.cancelled_count,
            },
            stage: task.stage,
            waiting_reason: task.waiting_reason,
            dependency_key: task.dependency_key,
            next_attempt_at: task.next_attempt_at,
            failure_stage: task.failure_stage,
            error_summary: task.error_summary,
            lease_expires_at: consistency.scoped_lease_until,
            created_at: task.created_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            updated_at: task.updated_at,
        },
        items,
        items_truncated,
        dependency_gates,
        consistency: TaskConsistencyReport {
            consistent: mismatches.is_empty(),
            current_item_id: consistency.current_item_id,
            mismatches,
            open_attempt_count: consistency.open_attempt_count,
            near_exhaustion_item_count: consistency.near_exhaustion_item_count,
        },
        observed_at: Utc::now(),
    })
}

fn task_attempt_view(attempt: &StoredTaskAttempt) -> TaskAttemptView {
    TaskAttemptView {
        attempt_id: attempt.attempt_id,
        attempt: attempt.attempt,
        status: attempt.status.clone(),
        retryable: attempt.retryable,
        failure_stage: attempt.failure_stage.clone(),
        error_message: attempt.error_message.clone(),
        started_at: attempt.started_at,
        finished_at: attempt.finished_at,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    use super::task_diagnose_response;
    use crate::db::{StoredTask, StoredTaskItem, TaskConsistencyRow};

    fn stored_task() -> StoredTask {
        StoredTask {
            id: Uuid::new_v4(),
            user_id: Some(1),
            group_id: None,
            kind: "text_batch".to_string(),
            status: "running".to_string(),
            origin: "manual".to_string(),
            group_path: None,
            source_key: None,
            total_count: 2,
            queued_count: 1,
            running_count: 1,
            waiting_count: 0,
            succeeded_count: 0,
            failed_count: 0,
            cancelled_count: 0,
            failure_stage: None,
            error_summary: None,
            stage: Some("processing".to_string()),
            waiting_reason: None,
            dependency_key: None,
            next_attempt_at: None,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            started_at: None,
            finished_at: None,
            updated_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 1).unwrap(),
            deleted_at: None,
            file_name: Some("report.pdf".to_string()),
            document_title: Some("Quarterly report".to_string()),
        }
    }

    fn stored_item(ordinal: i32) -> StoredTaskItem {
        StoredTaskItem {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            ordinal,
            status: "queued".to_string(),
            resource_id: None,
            file_id: None,
            stage: Some("processing".to_string()),
            waiting_reason: None,
            dependency_key: None,
            next_attempt_at: None,
            failure_stage: None,
            error_message: None,
            attempt_count: 0,
            retryable: true,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            started_at: None,
            finished_at: None,
            file_name: None,
            document_title: None,
        }
    }

    fn attempt_json(id: i64) -> serde_json::Value {
        json!({
            "attempt_id": id,
            "attempt": 1,
            "status": "running",
            "retryable": true,
            "failure_stage": null,
            "error_message": null,
            "started_at": "2026-01-01T00:00:00Z",
            "finished_at": null
        })
    }

    fn consistency_row(
        item_diagnostics: serde_json::Value,
        mismatch_fields: serde_json::Value,
    ) -> TaskConsistencyRow {
        TaskConsistencyRow {
            parent_count: 1,
            active_parent_count: 1,
            dependency_waiting_parent_count: 0,
            parent_status_counts: serde_json::json!([]),
            running_parent_without_running_item_count: 0,
            lease_without_running_item_count: 0,
            open_attempt_count: 1,
            near_exhaustion_item_count: 0,
            oldest_admitted_at: None,
            parent_item_mismatch_count: 0,
            live_running_count: 0,
            current_item_id: None,
            scoped_lease_until: None,
            mismatch_fields,
            item_diagnostics,
        }
    }

    /// An item with no diagnostics entry still projects. The two reads are
    /// independent, so a missing entry must degrade one item's detail, never
    /// drop the item from the response.
    #[test]
    fn diagnose_keeps_an_item_whose_diagnostics_are_absent() {
        let item = stored_item(0);
        let response = task_diagnose_response(
            stored_task(),
            vec![item.clone()],
            false,
            consistency_row(json!([]), json!([])),
            Vec::new(),
        )
        .expect("diagnose");
        assert_eq!(response.items.len(), 1);
        let projected = &response.items[0];
        assert_eq!(projected.item_id, item.id);
        assert_eq!(projected.ordinal, 0);
        assert!(projected.lease_expires_at.is_none());
        assert!(projected.active_attempt.is_none());
        assert!(projected.latest_attempt.is_none());
        assert!(response.consistency.consistent);
    }

    /// The attempt and lease detail is keyed by item id, not by position: the
    /// items arrive in ordinal order while the diagnostics array is built by
    /// the same ordering, so a mismatch would silently mislabel an attempt.
    #[test]
    fn diagnose_keys_attempt_forensics_by_item_id() {
        let first = stored_item(0);
        let second = stored_item(1);
        let response = task_diagnose_response(
            stored_task(),
            vec![first.clone(), second.clone()],
            false,
            consistency_row(
                json!([
                    {
                        "item_id": first.id,
                        "ordinal": 0,
                        "lease_expires_at": "2026-01-01T00:05:00Z",
                        "active_attempt": attempt_json(7),
                        "latest_attempt": attempt_json(7)
                    },
                    {
                        "item_id": second.id,
                        "ordinal": 1,
                        "lease_expires_at": null,
                        "active_attempt": null,
                        "latest_attempt": null
                    }
                ]),
                json!([]),
            ),
            Vec::new(),
        )
        .expect("diagnose");
        assert_eq!(
            response.items[0]
                .active_attempt
                .as_ref()
                .map(|attempt| attempt.attempt_id),
            Some(7),
            "the open attempt belongs to the item it was read for"
        );
        assert!(response.items[0].lease_expires_at.is_some());
        assert!(response.items[1].active_attempt.is_none());
    }

    /// The verdict names the disagreeing fields instead of only reporting a
    /// boolean, and a named mismatch means `consistent` is false.
    #[test]
    fn diagnose_reports_the_disagreeing_parent_fields() {
        let response = task_diagnose_response(
            stored_task(),
            vec![stored_item(0)],
            false,
            consistency_row(json!([]), json!(["running_count", "stage"])),
            Vec::new(),
        )
        .expect("diagnose");
        assert!(!response.consistency.consistent);
        assert_eq!(response.consistency.mismatches, ["running_count", "stage"]);
    }

    /// A malformed diagnostics payload is an error, not a silently empty
    /// response: diagnose exists to be trusted, so it must not report "no
    /// problems" when it could not read the problems.
    #[test]
    fn diagnose_rejects_unreadable_consistency_payloads() {
        let error = task_diagnose_response(
            stored_task(),
            vec![stored_item(0)],
            false,
            consistency_row(json!({"not": "an array"}), json!([])),
            Vec::new(),
        )
        .expect_err("malformed diagnostics must fail");
        assert!(
            error.to_string().contains("item diagnostics"),
            "the error must name what could not be read: {error}"
        );
        task_diagnose_response(
            stored_task(),
            vec![stored_item(0)],
            false,
            consistency_row(json!([]), json!("not an array of names")),
            Vec::new(),
        )
        .expect_err("malformed mismatches must fail");
    }
}
