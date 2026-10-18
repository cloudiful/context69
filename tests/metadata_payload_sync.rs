//! Pure contract tests for the automatic payload-mirror worker (issue #910 S1).
//!
//! These cover the retry curve, the lease/timeout ordering, and the exact SQL
//! fencing without a database or a Qdrant instance. Database and transport
//! behaviour is exercised only by the ignored cases registered for the
//! post-checkpoint verification.

use std::time::Duration;

use anyhow::anyhow;
use context69::qdrant_index::metadata_retirement::EXTRACTION_PAYLOAD_KEY;
use context69::services::metadata_payload_sync::{
    EXTERNAL_OPERATION_TIMEOUT, FALLBACK_INTERVAL, LEASE_DURATION, MAX_JOBS_PER_PASS, retry_delay,
    sanitize_failure_reason,
};

const CLAIM_SQL: &str = include_str!("../src/sql/db/metadata_payload_sync/claim.sql");
const COMPLETE_SQL: &str = include_str!("../src/sql/db/metadata_payload_sync/complete.sql");
const FAIL_SQL: &str = include_str!("../src/sql/db/metadata_payload_sync/fail.sql");
const POINTS_SQL: &str = include_str!("../src/sql/db/metadata_payload_sync/document_points.sql");

#[path = "metadata_payload_sync/database.rs"]
mod database;

#[test]
fn retry_curve_doubles_then_caps_without_discarding_the_job() {
    assert_eq!(retry_delay(1), Duration::from_secs(5));
    assert_eq!(retry_delay(2), Duration::from_secs(10));
    assert_eq!(retry_delay(3), Duration::from_secs(20));
    assert_eq!(retry_delay(4), Duration::from_secs(40));
    assert_eq!(retry_delay(5), Duration::from_secs(80));
    assert_eq!(retry_delay(6), Duration::from_secs(160));
    assert_eq!(retry_delay(7), Duration::from_secs(300));
    assert_eq!(retry_delay(60), Duration::from_secs(300));
    assert_eq!(retry_delay(u32::MAX), Duration::from_secs(300));
}

#[test]
fn the_lease_outlives_one_external_operation_and_the_pass_is_bounded() {
    assert!(
        EXTERNAL_OPERATION_TIMEOUT < LEASE_DURATION,
        "one external operation must finish before its lease expires"
    );
    assert_eq!(LEASE_DURATION, Duration::from_secs(60));
    assert_eq!(EXTERNAL_OPERATION_TIMEOUT, Duration::from_secs(20));
    assert_eq!(FALLBACK_INTERVAL, Duration::from_secs(5));
    assert_eq!(MAX_JOBS_PER_PASS, 16);
}

#[test]
fn the_worker_targets_exactly_the_retired_qdrant_payload_key() {
    assert_eq!(EXTRACTION_PAYLOAD_KEY, "metadata_index.extraction");
}

#[test]
fn failure_reasons_stay_one_bounded_line() {
    let error = anyhow!("bad\nline\twith\u{0}control").context("outer context");
    let reason = sanitize_failure_reason(&error);
    assert_eq!(reason, "outer context: bad line with control");
    assert!(!reason.contains('\n'));

    let long = anyhow!("{}", "x".repeat(2000));
    let reason = sanitize_failure_reason(&long);
    assert_eq!(reason.chars().count(), 512);
}

#[test]
fn claim_is_skip_locked_and_leases_one_due_row() {
    for fragment in [
        "completed_at IS NULL",
        "next_attempt_at <= now()",
        "lease_expires_at IS NULL OR lease_expires_at <= now()",
        "ORDER BY next_attempt_at, id",
        "LIMIT 1",
        "FOR UPDATE SKIP LOCKED",
        "lease_token = $1",
        "lease_expires_at = now() + make_interval(secs => ($2)::int)",
        "RETURNING sync.id, sync.document_id, sync.payload_key, sync.attempts, sync.lease_token",
    ] {
        assert!(
            CLAIM_SQL.contains(fragment),
            "claim contract missing: {fragment}"
        );
    }
}

#[test]
fn completion_and_failure_are_fenced_by_the_current_lease() {
    for fragment in [
        "completed_at = now()",
        "completed_at IS NULL",
        "lease_token = $2",
        "lease_token = NULL",
        "lease_expires_at = NULL",
    ] {
        assert!(
            COMPLETE_SQL.contains(fragment),
            "complete contract missing: {fragment}"
        );
    }

    for fragment in [
        "attempts = attempts + 1",
        "next_attempt_at = now() + make_interval(secs => ($3)::int)",
        "last_error = $4",
        "completed_at IS NULL",
        "lease_token = $2",
        "lease_token = NULL",
        "lease_expires_at = NULL",
    ] {
        assert!(
            FAIL_SQL.contains(fragment),
            "fail contract missing: {fragment}"
        );
    }
}

#[test]
fn point_enumeration_covers_originals_and_every_translation_generation() {
    for fragment in [
        "d.record_hash",
        "FROM context69.document_chunks chunk",
        "FROM context69.document_translation_chunks translation",
        "array_agg",
        "ARRAY[]::uuid[]",
        "WHERE d.id = $1",
    ] {
        assert!(
            POINTS_SQL.contains(fragment),
            "point enumeration missing: {fragment}"
        );
    }
}
