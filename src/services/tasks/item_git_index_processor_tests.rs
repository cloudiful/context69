//! Processor-level tests for the durable Git snapshot index task (3C1).
//!
//! The provider is always the in-memory `FakeProvider`; the DB-backed cases
//! use the authorized `.env` test fixture, read what they need, and delete the
//! group they created before asserting, so a failing assertion leaves no rows
//! behind. No migration, no `db_init`, and no live provider call ever runs.

use serde_json::json;
use uuid::Uuid;

use context69_contracts::TaskKind;

use crate::contracts::sources::GitIndexStatus;
use crate::domain_errors::DomainError;
use crate::services::git_repository::limits::GitIndexLimits;
use crate::services::git_repository::support::{FakeProvider, Fixture, binary_spec, sha};
use crate::services::tasks::item_processors::ProcessResult;

use super::{
    GIT_INDEX_STAGE, GitIndexPlan, is_bounded_retry, parse_repository_key, plan_git_index,
    run_git_index_snapshot,
};

#[test]
fn git_index_kind_serializes_stably() {
    assert_eq!(TaskKind::GitIndex.as_str(), "git_index");
    assert_eq!(
        serde_json::to_string(&TaskKind::GitIndex).unwrap(),
        "\"git_index\""
    );
    let decoded: TaskKind = serde_json::from_str("\"git_index\"").unwrap();
    assert_eq!(decoded, TaskKind::GitIndex);
}

#[test]
fn repository_key_payload_is_required_and_bounded() {
    let key = Uuid::new_v4();
    assert!(parse_repository_key(&json!({})).is_err());
    assert!(parse_repository_key(&json!({"repository_key": 7})).is_err());
    assert!(parse_repository_key(&json!({"repository_key": "not-a-uuid"})).is_err());
    assert_eq!(
        parse_repository_key(&json!({"repository_key": key.to_string()})).unwrap(),
        key
    );
    assert_eq!(
        parse_repository_key(&json!({"repository_key": format!("  {key}  ")})).unwrap(),
        key
    );
}

#[test]
fn plan_rejects_bad_stage_group_and_payload_terminally() {
    let key = Uuid::new_v4();
    let payload = json!({"repository_key": key.to_string()});

    for plan in [
        plan_git_index(Some(1), &payload, "nonsense"),
        plan_git_index(None, &payload, GIT_INDEX_STAGE),
        plan_git_index(Some(1), &json!({}), GIT_INDEX_STAGE),
    ] {
        match plan {
            Err(ProcessResult::Failed { retryable, .. }) => assert!(!retryable),
            _ => panic!("invalid git index request must fail terminally"),
        }
    }

    match plan_git_index(Some(7), &payload, GIT_INDEX_STAGE) {
        Ok(GitIndexPlan::Run {
            group_id,
            repository_key,
        }) => {
            assert_eq!(group_id, 7);
            assert_eq!(repository_key, key);
        }
        _ => panic!("valid request must plan a run"),
    }
    assert!(matches!(
        plan_git_index(None, &payload, "finalize"),
        Ok(GitIndexPlan::Finalize)
    ));
}

#[test]
fn transient_failures_retry_and_typed_terminal_failures_do_not() {
    let retryable: anyhow::Error = DomainError::upstream_error("git_http_status_500").into();
    assert!(is_bounded_retry(&retryable));

    let stale: anyhow::Error = DomainError::conflict("git_index_stale_target").into();
    assert!(is_bounded_retry(&stale));

    // Untyped infrastructure failures retry, but the worker attempt cap bounds
    // them instead of allowing an unbounded loop.
    assert!(is_bounded_retry(&anyhow::anyhow!(
        "database connection reset"
    )));

    for terminal in [
        DomainError::not_found("git_index_source_missing"),
        DomainError::invalid_argument("git_index_repository_key_invalid"),
        DomainError::payload_too_large("git_index_file_limit"),
        DomainError::internal("git_index_manifest_entry_missing"),
    ] {
        let error: anyhow::Error = terminal.into();
        assert!(!is_bounded_retry(&error));
    }
}

#[tokio::test]
async fn successful_snapshot_advances_to_finalize_and_activates() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let provider = FakeProvider::new(binary_spec(&sha(0x51)));
    let result = run_git_index_snapshot(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        &provider,
        GitIndexLimits::default(),
    )
    .await;
    let reads = fixture.reads(None).await;
    fixture.cleanup().await;

    assert!(matches!(
        result,
        ProcessResult::Progressed { next } if next == "finalize"
    ));
    assert!(
        reads.active.is_some(),
        "snapshot must activate a generation"
    );
    let source = reads.source.expect("source remains readable");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(reads.generations.len(), 1);
}

#[tokio::test]
async fn missing_or_foreign_repository_fails_terminally() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let provider = FakeProvider::new(binary_spec(&sha(0x52)));
    let result = run_git_index_snapshot(
        fixture.db(),
        fixture.group_id,
        Uuid::new_v4(),
        &provider,
        GitIndexLimits::default(),
    )
    .await;
    let reads = fixture.reads(None).await;
    fixture.cleanup().await;

    assert!(matches!(
        result,
        ProcessResult::Failed {
            retryable: false,
            ..
        }
    ));
    assert!(reads.active.is_none());
    assert!(reads.generations.is_empty());
}

#[tokio::test]
async fn failed_blob_fetch_retries_and_keeps_the_prior_generation() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let prior = fixture.start_prior_generation().await;
    let mut spec = binary_spec(&sha(0x53));
    spec.fail_blob = Some(sha(0xb1));
    let provider = FakeProvider::new(spec);
    let result = run_git_index_snapshot(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        &provider,
        GitIndexLimits::default(),
    )
    .await;
    let reads = fixture.reads(None).await;
    fixture.cleanup().await;

    assert!(matches!(
        result,
        ProcessResult::Failed {
            retryable: true,
            ..
        }
    ));
    assert_eq!(reads.active, Some(prior), "prior generation stays serving");
}
