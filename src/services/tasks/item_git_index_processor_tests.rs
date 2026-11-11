//! Processor-level tests for the durable Git snapshot index task (3C1).
//!
//! The provider is always the in-memory `FakeProvider`; the DB-backed cases
//! use the authorized `.env` test fixture, read what they need, and delete the
//! group they created before asserting, so a failing assertion leaves no rows
//! behind. No migration, no `db_init`, and no live provider call ever runs.

use serde_json::json;
use uuid::Uuid;

use context69_contracts::TaskKind;

use crate::domain_errors::DomainError;
use crate::services::tasks::item_processors::ProcessResult;

use super::{
    FAILURE_CODE_SEPARATOR, GIT_INDEX_STAGE, GitIndexPlan, failure_code, is_bounded_retry,
    parse_repository_key, plan_git_index, terminal_failure,
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
            force_rebuild,
        }) => {
            assert_eq!(group_id, 7);
            assert_eq!(repository_key, key);
            assert!(!force_rebuild);
        }
        _ => panic!("valid request must plan a run"),
    }
    assert!(matches!(
        plan_git_index(None, &payload, "finalize"),
        Ok(GitIndexPlan::Finalize)
    ));

    let forced = json!({
        "repository_key": key.to_string(),
        "force_rebuild": true
    });
    assert!(matches!(
        plan_git_index(Some(7), &forced, GIT_INDEX_STAGE),
        Ok(GitIndexPlan::Run {
            force_rebuild: true,
            ..
        })
    ));
    let invalid = json!({
        "repository_key": key.to_string(),
        "force_rebuild": "yes"
    });
    assert!(matches!(
        plan_git_index(Some(7), &invalid, GIT_INDEX_STAGE),
        Err(ProcessResult::Failed {
            retryable: false,
            ..
        })
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

#[test]
fn terminal_failure_codes_are_stable_and_never_carry_provider_detail() {
    let safe: anyhow::Error = DomainError::not_found("git_index_source_missing").into();
    assert_eq!(failure_code(&safe), "git_index_source_missing");

    let detailed: anyhow::Error =
        DomainError::upstream_error("provider body token=secret https://internal.example/x").into();
    assert_eq!(failure_code(&detailed), "git_index_upstream_error");

    let untyped: anyhow::Error = anyhow::anyhow!("connection string postgres://secret");
    assert_eq!(failure_code(&untyped), "git_index_failed");

    match terminal_failure(GIT_INDEX_STAGE, "git_index_source_missing") {
        ProcessResult::Failed {
            message,
            retryable: false,
            ..
        } => assert_eq!(
            message,
            format!("git_index_source_missing{FAILURE_CODE_SEPARATOR}git_index_source_missing")
        ),
        _ => panic!("a terminal failure is not retryable"),
    }
    match terminal_failure(GIT_INDEX_STAGE, "provider said secret=abc") {
        ProcessResult::Failed { message, .. } => {
            assert!(message.starts_with(&format!(
                "git_index_terminal_failure{FAILURE_CODE_SEPARATOR}"
            )));
            assert!(!message.starts_with("provider"));
        }
        _ => panic!("a terminal failure is always reported"),
    }
}
