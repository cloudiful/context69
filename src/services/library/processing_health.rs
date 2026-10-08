//! The `/healthz` library-processing projection (issue 702 P3).
//!
//! `dependency_runtime.rs` decides *when* a gate transition happens and runs the
//! whole-queue scan; this module owns the last step — turning the stored gate
//! rows and the parent/item consistency row into the wire shape the health
//! payload reports. Keeping the mapping here means the gate response shape and
//! the parent consistency gauges are defined once, next to each other.

use anyhow::Result;
use chrono::{DateTime, Utc};
use context69_contracts::{
    LibraryDependencyGateResponse, LibraryProcessingMetric, TaskQueueConsistencyHealth,
};
use serde_json::Value;

use super::LibraryService;

impl LibraryService {
    /// Every dependency gate in its wire shape, without the queue scan.
    ///
    /// The task diagnose endpoint reports the gates an item can be waiting on,
    /// and it must not pay for the whole-queue snapshot (or be degraded by it)
    /// to learn that.
    pub(crate) async fn dependency_gate_snapshot(
        &self,
    ) -> Result<Vec<LibraryDependencyGateResponse>> {
        Ok(dependency_gate_responses(
            self.store.list_dependency_gates().await?,
        ))
    }
}

/// Project stored gate rows into their wire shape. The failure count is clamped
/// because a negative value is a corrupt row, not a reportable gauge.
pub(super) fn dependency_gate_responses(
    gates: Vec<crate::library_store::DependencyGateRecord>,
) -> Vec<LibraryDependencyGateResponse> {
    gates
        .into_iter()
        .map(|gate| LibraryDependencyGateResponse {
            dependency_key: gate.dependency_key,
            state: gate.state,
            failure_count: u32::try_from(gate.failure_count.max(0)).unwrap_or(u32::MAX),
            next_probe_at: gate.next_probe_at,
            last_error: gate.last_error,
            last_transition_at: gate.last_transition_at,
            last_success_at: gate.last_success_at,
        })
        .collect()
}

/// Project the whole-queue consistency row into the parent health gauges.
///
/// Counts are clamped the same way the queue snapshot clamps its own, so one
/// corrupt row degrades a gauge instead of failing `/healthz`.
pub(super) fn parent_consistency_health(
    consistency: &crate::db::TaskConsistencyRow,
    now: DateTime<Utc>,
) -> TaskQueueConsistencyHealth {
    TaskQueueConsistencyHealth {
        parent_count: non_negative_count(consistency.parent_count),
        active_parent_count: non_negative_count(consistency.active_parent_count),
        dependency_waiting_parent_count: non_negative_count(
            consistency.dependency_waiting_parent_count,
        ),
        parent_status_counts: parse_processing_metrics(consistency.parent_status_counts.clone()),
        running_parent_without_running_item_count: non_negative_count(
            consistency.running_parent_without_running_item_count,
        ),
        lease_without_running_item_count: non_negative_count(
            consistency.lease_without_running_item_count,
        ),
        open_attempt_count: non_negative_count(consistency.open_attempt_count),
        near_exhaustion_item_count: non_negative_count(consistency.near_exhaustion_item_count),
        oldest_admitted_age_seconds: queue_age_seconds(consistency.oldest_admitted_at, now),
        parent_item_mismatch_count: non_negative_count(consistency.parent_item_mismatch_count),
    }
}

fn non_negative_count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn queue_age_seconds(timestamp: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<u64> {
    timestamp.map(|timestamp| now.signed_duration_since(timestamp).num_seconds().max(0) as u64)
}

fn parse_processing_metrics(value: Value) -> Vec<LibraryProcessingMetric> {
    serde_json::from_value(value).unwrap_or_else(|error| {
        tracing::warn!(%error, "library processing health metrics are unreadable");
        Vec::new()
    })
}

/// Report a parent/item invariant breach on the health path.
///
/// A non-zero mismatch or orphan count is an internal-invariant breach, not a
/// user error, so it is logged with the disagreeing field names: an operator
/// gets an actionable line without needing a diagnose call.
pub(super) fn log_parent_consistency_breach(
    consistency: &crate::db::TaskConsistencyRow,
    gauges: &TaskQueueConsistencyHealth,
) {
    if gauges.parent_item_mismatch_count == 0 && gauges.lease_without_running_item_count == 0 {
        return;
    }
    tracing::warn!(
        target: "task_lifecycle",
        parent_item_mismatch_count = gauges.parent_item_mismatch_count,
        mismatch_fields = ?consistency.mismatch_fields,
        running_parent_without_running_item_count =
            gauges.running_parent_without_running_item_count,
        lease_without_running_item_count = gauges.lease_without_running_item_count,
        "task parent projection disagrees with its items"
    );
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::{dependency_gate_responses, non_negative_count, parent_consistency_health};
    use crate::db::TaskConsistencyRow;

    fn consistency_row() -> TaskConsistencyRow {
        TaskConsistencyRow {
            parent_count: 3,
            active_parent_count: 1,
            dependency_waiting_parent_count: 1,
            parent_status_counts: json!([{"key": "waiting", "count": 1}]),
            running_parent_without_running_item_count: 0,
            lease_without_running_item_count: 0,
            open_attempt_count: 1,
            near_exhaustion_item_count: 2,
            oldest_admitted_at: None,
            parent_item_mismatch_count: 0,
            live_running_count: 1,
            current_item_id: None,
            scoped_lease_until: None,
            mismatch_fields: json!([]),
            item_diagnostics: json!([]),
        }
    }

    fn gate(
        key: &str,
        state: &str,
        failure_count: i32,
    ) -> crate::library_store::DependencyGateRecord {
        crate::library_store::DependencyGateRecord {
            dependency_key: key.to_string(),
            state: state.to_string(),
            failure_count,
            next_probe_at: None,
            probe_lease_expires_at: None,
            last_error: None,
            probe_lease_token: None,
            last_transition_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            last_success_at: None,
        }
    }

    /// A corrupt row must degrade one gauge, never the whole health probe.
    #[test]
    fn negative_counts_and_unreadable_metrics_degrade_to_zero_and_empty() {
        let mut row = consistency_row();
        row.parent_count = -1;
        row.parent_status_counts = json!("not a metric array");
        let gauges =
            parent_consistency_health(&row, Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(gauges.parent_count, 0);
        assert!(gauges.parent_status_counts.is_empty());
        assert_eq!(
            gauges.active_parent_count, 1,
            "an unrelated gauge must still be reported"
        );
        assert_eq!(non_negative_count(-5), 0);
        assert_eq!(non_negative_count(5), 5);
    }

    /// Every gate state reaches the payload, including the half-open recovery
    /// state P2 added.
    #[test]
    fn gate_states_reach_the_wire_shape() {
        let responses = dependency_gate_responses(vec![
            gate("s3", "closed", 0),
            gate("docling", "half_open", -1),
            gate("qdrant", "open", 3),
        ]);
        assert_eq!(
            responses
                .iter()
                .map(|g| g.state.as_str())
                .collect::<Vec<_>>(),
            ["closed", "half_open", "open"]
        );
        assert_eq!(
            responses[1].failure_count, 0,
            "a negative stored failure count is a corrupt row, not a gauge"
        );
        assert_eq!(responses[2].failure_count, 3);
    }
}
