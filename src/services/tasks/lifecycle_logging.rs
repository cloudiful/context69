//! The task lifecycle log vocabulary.
//!
//! One identity, recorded once, for every task lifecycle transition. The
//! dispatcher (`dispatcher.rs`) decides when an item is claimed and when a lease
//! expires; the worker (`runtime_driver.rs`) reports what happened to that claim.
//! Both sides go through [`lifecycle_span`], so one transition is greppable by
//! `task_id`/`item_id`/`ordinal`/`attempt`/`stage`/`status` no matter which side
//! emitted it, and a transition logged deep inside the item processors inherits
//! the same identity without re-listing the fields.

use chrono::{DateTime, Utc};
use tracing::{Span, debug, info, warn};

/// The identity every task lifecycle transition is recorded under.
///
/// The worker enters this span for the whole claim, so the per-stage entry
/// events emitted by the item processors carry the claim's identity without
/// each one re-declaring it. No field here is an item payload, a lease token, or
/// a secret: `ordinal`, `stage`, and `status` are names, and the two error
/// fields carry the messages the item rows already expose.
pub(super) fn lifecycle_span(item: &crate::db::ClaimedItem, stage_entry: &'static str) -> Span {
    // Declared empty and filled by hand: a span's field set is fixed in the
    // callsite, while these values are only known at runtime.
    let span = tracing::info_span!(
        target: "task_lifecycle",
        "task item claim",
        task_id = tracing::field::Empty,
        item_id = tracing::field::Empty,
        ordinal = tracing::field::Empty,
        attempt = tracing::field::Empty,
        attempt_id = tracing::field::Empty,
        kind = tracing::field::Empty,
        stage = tracing::field::Empty,
        stage_entry = tracing::field::Empty,
    );
    span.record("task_id", tracing::field::display(item.task_id));
    span.record("item_id", tracing::field::display(item.id));
    span.record("ordinal", item.ordinal);
    span.record("attempt", item.attempt_count);
    span.record("attempt_id", item.attempt_id);
    span.record("kind", item.kind.as_str());
    span.record("stage", item.stage.as_deref().unwrap_or("processing"));
    span.record("stage_entry", stage_entry);
    span
}

/// An item was admitted by the claim statement.
///
/// Recorded before the local worker slot is taken: a claim that cannot get a slot
/// on this replica is still a claim the database made, and this is the line an
/// operator correlates with the item lease. The ordinal is the one the claim
/// statement returned, so it cannot disagree with the row being processed.
pub(super) fn log_claim(item: &crate::db::ClaimedItem) {
    info!(
        target: "task_lifecycle",
        task_id = %item.task_id,
        item_id = %item.id,
        ordinal = item.ordinal,
        attempt = item.attempt_count,
        attempt_id = item.attempt_id,
        stage = item.stage.as_deref().unwrap_or("processing"),
        kind = %item.kind,
        status = "running",
        "task item claimed"
    );
}

/// A wake found nothing claimable. Debug, because it is the common case: a submit
/// notification on an already-drained queue, or a coalesced retry.
pub(super) fn log_no_claimable_item(parent_capacity: usize, available_slots: usize) {
    debug!(
        target: "task_lifecycle",
        parent_capacity,
        available_slots,
        "task dispatcher found no claimable item"
    );
}

/// What one dispatch pass claimed. Info when work moved, debug when the pass was
/// a no-op, so an operator can tell "the queue drained" from "the dispatcher is
/// idle" without raising the level of the common case.
pub(super) fn log_claims_dispatched(
    claimed_total: usize,
    parent_capacity: usize,
    available_slots: usize,
) {
    let inflight_count = parent_capacity.saturating_sub(available_slots);
    if claimed_total > 0 {
        info!(
            target: "task_lifecycle",
            claimed_total,
            parent_capacity,
            inflight_count,
            available_slots,
            "task dispatcher claimed items"
        );
    } else {
        debug!(
            target: "task_lifecycle",
            claimed_total,
            parent_capacity,
            inflight_count,
            available_slots,
            "task dispatcher state"
        );
    }
}

/// What the claim maintenance converged.
///
/// Terminal convergence is info; an interrupted attempt or a revoked parent
/// admission lease is warn, because both mean a worker died still holding a
/// claim, and those are the two an operator has to act on.
pub(super) fn log_maintenance_outcome(outcome: &crate::db::ClaimMaintenanceOutcome) {
    if outcome.exhausted_items
        + outcome.exhausted_files
        + outcome.exhausted_tasks
        + outcome.expired_attempts
        > 0
    {
        info!(
            target: "task_lifecycle",
            exhausted_items = outcome.exhausted_items,
            exhausted_files = outcome.exhausted_files,
            exhausted_tasks = outcome.exhausted_tasks,
            expired_attempts = outcome.expired_attempts,
            "task claim maintenance converged terminal state"
        );
    }
    if outcome.expired_attempts > 0 {
        warn!(
            target: "task_lifecycle",
            expired_attempts = outcome.expired_attempts,
            "task item lease expired; attempts interrupted and items reclaimable"
        );
    }
    if outcome.revoked_parent_leases > 0 {
        // A parent slot released while its worker item lease was gone is the
        // orphan-lease case: the slot was occupied by work nobody owned. The
        // next claim re-admits it.
        warn!(
            target: "task_lifecycle",
            renewed_parent_leases = outcome.renewed_parent_leases,
            revoked_parent_leases = outcome.revoked_parent_leases,
            "task parent admission lease revoked; parent lease without a live item"
        );
    } else if outcome.renewed_parent_leases > 0 {
        debug!(
            target: "task_lifecycle",
            renewed_parent_leases = outcome.renewed_parent_leases,
            revoked_parent_leases = outcome.revoked_parent_leases,
            "task parent admission leases converged"
        );
    }
}

/// Maintenance is best-effort: a transient database error must not stall the
/// dispatcher loop, and the next recovery tick retries it.
pub(super) fn log_maintenance_failed(error: &anyhow::Error) {
    warn!(target: "task_lifecycle", %error, "task claim maintenance failed; continuing");
}

/// One attempt ended, whatever ended it. The fenced statement that parks or
/// finishes the item also closes its attempt row, so this is the one place the
/// attempt forensics become observable.
pub(super) fn log_attempt(outcome: &'static str) {
    info!(target: "task_lifecycle", outcome, "task item attempt finished");
}

/// The item reached a terminal status; the parent recompute committed with it in
/// the same transaction.
pub(super) fn log_item_finished(status: &'static str) {
    info!(target: "task_lifecycle", status, "task item finished");
}

/// A dependency-gate park is warn-level because it is the one wait an operator
/// has to act on; every other wait is ordinary backoff.
pub(super) fn log_dependency_wait(
    reason: &str,
    dependency_key: &str,
    next_attempt_at: DateTime<Utc>,
    message: Option<&str>,
) {
    warn!(
        target: "task_lifecycle",
        status = "waiting",
        reason,
        dependency_key,
        next_attempt_at = %next_attempt_at,
        message = ?message,
        "task item parked on a dependency gate"
    );
}

pub(super) fn log_waiting(reason: &str, next_attempt_at: DateTime<Utc>, message: Option<&str>) {
    info!(
        target: "task_lifecycle",
        status = "waiting",
        reason,
        next_attempt_at = %next_attempt_at,
        message = ?message,
        "task item waiting"
    );
}

pub(super) fn log_retry_scheduled(next_attempt_at: DateTime<Utc>) {
    info!(
        target: "task_lifecycle",
        status = "waiting",
        reason = "backoff",
        next_attempt_at = %next_attempt_at,
        "task item retry scheduled"
    );
}

/// The parent reached a terminal status. Reported after the recompute, so the
/// counts in the log are the ones that transition committed.
pub(super) fn log_task_finished(task: &crate::db::StoredTask) {
    info!(
        target: "task_lifecycle",
        task_id = %task.id,
        status = %task.status,
        kind = %task.kind,
        total = task.total_count,
        succeeded = task.succeeded_count,
        failed = task.failed_count,
        cancelled = task.cancelled_count,
        stage = ?task.stage,
        failure_stage = ?task.failure_stage,
        error_summary = ?task.error_summary,
        "task finished"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::lifecycle_span;
    use crate::db::ClaimedItem;

    fn claimed_item() -> ClaimedItem {
        ClaimedItem {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            ordinal: 7,
            attempt_count: 3,
            lease_token: Uuid::new_v4(),
            attempt_id: 77,
            payload: serde_json::json!({"content": "must never be logged"}),
            file_id: None,
            stage: Some("processing".to_string()),
            input_storage_object_id: None,
            kind: "text_batch".to_string(),
            group_id: None,
            group_path: None,
            source_key: None,
        }
    }

    /// The field names a span declares. Reading them from the span metadata is
    /// what an operator sees on every event recorded inside it, and it needs no
    /// subscriber installed.
    fn declared_fields(item: &ClaimedItem, stage_entry: &'static str) -> Vec<String> {
        lifecycle_span(item, stage_entry)
            .metadata()
            .expect("a span always has metadata")
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect()
    }

    /// Collects the formatted output of one subscriber, so the assertions read
    /// the line an operator actually sees rather than a hand-built field map.
    #[derive(Clone, Default)]
    struct Rendered(Arc<std::sync::Mutex<String>>);

    impl std::io::Write for Rendered {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            let mut rendered = self.0.lock().expect("rendered");
            rendered.push_str(&String::from_utf8_lossy(buffer));
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Rendered {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// The claim span is what makes a transition correlatable to item position
    /// and to the stage the claim enters.
    #[test]
    fn lifecycle_span_declares_identity_and_the_real_stage_entry() {
        let fields = declared_fields(&claimed_item(), "storage");
        for required in [
            "task_id",
            "item_id",
            "ordinal",
            "attempt",
            "attempt_id",
            "kind",
            "stage",
            "stage_entry",
        ] {
            assert!(
                fields.iter().any(|field| field == required),
                "the lifecycle span must record `{required}`, got {fields:?}"
            );
        }
    }

    /// A span is an observability surface, so it must never carry the item
    /// payload or a lease token: the field names are the contract an operator
    /// sees, and neither of those may appear in it.
    #[test]
    fn lifecycle_span_never_declares_payload_or_lease_token_fields() {
        let fields = declared_fields(&claimed_item(), "download");
        for forbidden in ["payload", "lease_token", "content", "secret"] {
            assert!(
                !fields.iter().any(|field| field == forbidden),
                "the lifecycle span must not record `{forbidden}`, got {fields:?}"
            );
        }
    }

    /// The end-to-end proof: an event logged inside the claim span — such as
    /// the per-stage entry event the item processors emit — carries item
    /// position and the real entry stage without naming them itself.
    #[test]
    fn events_inside_the_span_inherit_ordinal_and_stage_entry() {
        let rendered = Rendered::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(rendered.clone())
            .finish();
        let item = claimed_item();
        assert_eq!(
            item.payload["content"].as_str(),
            Some("must never be logged"),
            "the fixture must carry a payload that would leak if it were logged"
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = super::lifecycle_span(&item, "storage");
            let _entered = span.enter();
            tracing::info!(entered_stage = "storage", "task item entered stage");
        });

        let line = rendered.0.lock().expect("rendered").clone();
        for expected in [
            "ordinal=7",
            "attempt=3",
            "task_id=",
            "item_id=",
            "stage_entry=\"storage\"",
            "entered_stage=\"storage\"",
        ] {
            assert!(
                line.contains(expected),
                "an event inside the claim span must report `{expected}`: {line}"
            );
        }
        assert!(
            !line.contains("must never be logged"),
            "an inherited lifecycle event must not drag in the payload: {line}"
        );
    }
}
