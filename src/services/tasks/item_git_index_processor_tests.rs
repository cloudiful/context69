//! Processor-level tests for the durable Git snapshot index task (3C1).
//!
//! The provider is always the in-memory `FakeProvider`; the DB-backed cases
//! use the authorized `.env` test fixture, read what they need, and delete the
//! group they created before asserting, so a failing assertion leaves no rows
//! behind. No migration, no `db_init`, and no live provider call ever runs.

use serde_json::json;
use uuid::Uuid;

use context69_contracts::TaskKind;

use crate::contracts::sources::{
    GitConnectionMode, GitIndexProfile, GitIndexStatus, GitProviderKind,
};
use crate::db::{NewGitProviderConnection, NewGitRepositorySource};
use crate::domain_errors::DomainError;
use crate::services::git_repository::limits::GitIndexLimits;
use crate::services::git_repository::support::{FakeProvider, Fixture, binary_spec, sha};
use crate::services::tasks::item_processors::ProcessResult;
use crate::services::{
    git_secrets::{GitSecretSlot, GitSecretTarget, GitSecretWrite, GitSecretWriter},
    secret_store,
};

use super::{
    FAILURE_CODE_SEPARATOR, GIT_INDEX_STAGE, GitIndexPlan, build_provider, failure_code,
    is_bounded_retry, parse_repository_key, plan_git_index, run_git_index_snapshot,
    terminal_failure,
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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn successful_snapshot_advances_to_finalize_and_activates() {
    let fixture = Fixture::setup().await;
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn missing_or_foreign_repository_fails_terminally() {
    let fixture = Fixture::setup().await;
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn token_connections_load_only_their_record_scoped_credential() {
    for (provider, connection_key, base_url, canonical_prefix) in [
        (
            GitProviderKind::Forgejo,
            "forgejo-main",
            "https://forgejo.example/api/v1",
            "https://forgejo.example/octo",
        ),
        (
            GitProviderKind::GitHub,
            "github-main",
            "https://api.github.com",
            "https://github.com/octo",
        ),
    ] {
        let fixture = Fixture::setup().await;
        let store = secret_store::build_unkeyed(fixture.db());
        let target = GitSecretTarget::Connection {
            group_id: fixture.group_id,
            connection_key: connection_key.to_string(),
        };
        let GitSecretWrite::Stored(secret_key) = GitSecretWriter::new(fixture.db().clone(), store)
            .seal(&target, GitSecretSlot::Credential, Some(b"pat-value"))
            .await
            .expect("seal credential reference")
        else {
            panic!("a supplied credential must be stored");
        };
        fixture
            .db()
            .insert_git_provider_connection(
                fixture.group_id,
                &NewGitProviderConnection {
                    connection_key: connection_key.to_string(),
                    provider,
                    mode: GitConnectionMode::Token,
                    display_name: format!("{provider:?}"),
                    base_url: base_url.to_string(),
                    credential_secret_key: Some(secret_key.as_str().to_string()),
                    webhook_secret_key: None,
                },
            )
            .await
            .expect("insert connection");
        let name = Uuid::new_v4().to_string();
        let bound = fixture
            .db()
            .upsert_git_repository_source(
                fixture.group_id,
                &NewGitRepositorySource {
                    connection_key: Some(connection_key.to_string()),
                    provider,
                    canonical_url: format!("{canonical_prefix}/{name}"),
                    owner: "octo".to_string(),
                    name,
                    default_branch: "main".to_string(),
                    target_ref: "refs/heads/main".to_string(),
                    pinned_commit_sha: None,
                    index_profile: GitIndexProfile::Lexical,
                    include_patterns: Vec::new(),
                    exclude_patterns: Vec::new(),
                    interval_seconds: None,
                    webhook_enabled: false,
                    enabled: true,
                    retention_count: 2,
                    created_by_user_id: None,
                },
            )
            .await
            .expect("bind source to connection");
        let (service, storage_root) =
            crate::services::tasks::item_pipeline_tests::support::build_service(fixture.db()).await;

        let loaded = build_provider(&service, &bound).await;
        sqlx::query_file!(
            "tests/sql/runtime_readiness/delete_internal_secret.sql",
            secret_key.as_str()
        )
        .execute(fixture.db().pool())
        .await
        .expect("remove the sealed credential");
        let uncredentialed = fixture
            .db()
            .get_git_repository_source(fixture.group_id, bound.repository_key)
            .await
            .expect("read the uncredentialed source")
            .expect("the fixture owns the source");
        let refused = build_provider(&service, &uncredentialed).await;
        fixture.cleanup().await;
        let _ = std::fs::remove_dir_all(storage_root);

        assert!(
            loaded.is_ok(),
            "{provider:?} token connection must load its sealed credential"
        );
        match refused {
            Err(ProcessResult::Failed {
                retryable: false,
                message,
                ..
            }) => {
                assert!(
                    message.contains("git_connection_credential_missing"),
                    "a missing credential is terminal and names no secret: {message}"
                );
                assert!(
                    !message.contains("pat-value"),
                    "the refusal must not reflect the credential: {message}"
                );
            }
            _ => panic!("a token connection without its credential must be refused"),
        }
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn failed_blob_fetch_retries_and_keeps_the_prior_generation() {
    let fixture = Fixture::setup().await;
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
