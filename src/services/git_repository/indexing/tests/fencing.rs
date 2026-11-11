//! Revision, acquisition, connection, and force-demand fencing.

use crate::contracts::sources::{
    GitConnectionMode, GitGenerationStatus, GitIndexProfile, GitIndexStatus, GitProviderKind,
};
use crate::db::{GitRepositorySettingsUpdate, NewGitProviderConnection};

use super::super::super::{limits::GitIndexLimits, support::*};
use super::super::{index_git_repository_snapshot, index_git_repository_snapshot_at_revision};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_changed_target_is_rejected_as_stale() {
    let fixture = Fixture::setup().await;
    let concurrent = sha(0xee);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa3),
        files: vec![("src/lib.rs", sha(0xc1), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: Some(SourceMutation {
            db: fixture.db().clone(),
            group_id: fixture.group_id,
            repository_key: fixture.repository_key,
            commit_sha: concurrent.clone(),
            action: SourceMutationAction::ObservedTarget,
        }),
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    fixture.cleanup().await;

    let error = result.expect_err("a changed target must not activate");
    assert!(
        error.to_string().contains("git_index_stale_target"),
        "{error}"
    );
    assert!(
        reads.active.is_none(),
        "a stale generation must not activate"
    );
    assert_eq!(
        reads
            .generations
            .first()
            .map(|generation| generation.status),
        Some(GitGenerationStatus::Failed)
    );
    let source = reads.source.expect("source still exists");
    assert_eq!(
        source.index_status,
        GitIndexStatus::Indexing,
        "a concurrent change owns the source; the stale attempt must not clobber it"
    );
    assert_eq!(
        source.checkpoint.target_commit_sha.as_deref(),
        Some(concurrent.as_str())
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn two_successive_branch_heads_advance_the_observed_target() {
    let fixture = Fixture::setup().await;
    let first_commit = sha(0xa6);
    let first = FakeProvider::new(FakeAcquirerSpec {
        commit: first_commit.clone(),
        files: vec![("src/one.rs", sha(0xd1), b"pub fn one() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let first_outcome = index_git_repository_snapshot(
        fixture.db(),
        &first,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the first branch head indexes");

    let second_commit = sha(0xa7);
    let second = FakeProvider::new(FakeAcquirerSpec {
        commit: second_commit.clone(),
        files: vec![("src/two.rs", sha(0xd2), b"pub fn two() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let second_outcome = index_git_repository_snapshot(
        fixture.db(),
        &second,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the advanced branch head indexes");

    let reads = fixture.reads(Some(second_outcome.generation_key)).await;
    fixture.cleanup().await;

    assert!(!second_outcome.no_op);
    assert_ne!(second_outcome.generation_key, first_outcome.generation_key);
    assert_eq!(second_outcome.commit_sha, second_commit);
    let source = reads.source.expect("source still exists");
    assert_eq!(
        source.version.commit_sha, None,
        "a branch target is never reinterpreted as a fixed pin"
    );
    assert_eq!(
        source.checkpoint.target_commit_sha.as_deref(),
        Some(second_commit.as_str())
    );
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(second_commit.as_str())
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_source_edit_during_acquisition_never_publishes_over_the_newer_state() {
    let actions = vec![
        SourceMutationAction::ExcludePatterns(vec!["docs/**".to_string()]),
        SourceMutationAction::Disable,
        SourceMutationAction::TargetRef("refs/heads/other".to_string()),
        SourceMutationAction::IndexProfile(GitIndexProfile::Hybrid),
        SourceMutationAction::ConnectionAttach,
    ];
    for action in actions {
        let fixture = Fixture::setup().await;
        let prior = fixture.start_prior_generation().await;
        let provider = FakeProvider::new(FakeAcquirerSpec {
            commit: sha(0xab),
            files: vec![("src/lib.rs", sha(0xd4), b"pub fn x() {}\n")],
            excluded: 0,
            too_large: 0,
            fail_blob: None,
            mutate: Some(SourceMutation {
                db: fixture.db().clone(),
                group_id: fixture.group_id,
                repository_key: fixture.repository_key,
                commit_sha: sha(0xac),
                action: action.clone(),
            }),
        });
        let result = index_git_repository_snapshot(
            fixture.db(),
            &provider,
            GitIndexLimits::default(),
            fixture.group_id,
            fixture.repository_key,
            None,
            false,
        )
        .await;
        let reads = fixture
            .reads(result.as_ref().ok().map(|outcome| outcome.generation_key))
            .await;
        fixture.cleanup().await;

        let error = result.expect_err("a newer source edit must reject the attempt");
        assert!(
            error.to_string().contains("git_index_stale_target"),
            "{error}"
        );
        assert_eq!(
            reads.active,
            Some(prior),
            "the previous active generation keeps serving after {action:?}"
        );
        let source = reads.source.expect("source still exists");
        assert_ne!(
            source.index_status,
            GitIndexStatus::Ready,
            "a stale attempt must not mark the newer source ready"
        );
        assert_ne!(
            source.checkpoint.indexed_commit_sha.as_deref(),
            Some(sha(0xab).as_str()),
            "a stale attempt must not advance the indexed checkpoint"
        );
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_connection_disabled_during_acquisition_never_publishes_the_old_credential() {
    let fixture = Fixture::setup().await;
    let connection_key = "paused-connection";
    fixture
        .db()
        .upsert_git_provider_connection(
            fixture.group_id,
            &NewGitProviderConnection {
                connection_key: connection_key.to_string(),
                provider: GitProviderKind::GitHub,
                mode: GitConnectionMode::Public,
                display_name: "Paused connection".to_string(),
                base_url: "https://github.com".to_string(),
                credential_secret_key: None,
                webhook_secret_key: None,
            },
        )
        .await
        .expect("seed the provider connection");
    fixture
        .attach_connection(connection_key)
        .await
        .expect("attach the connection");
    let prior = fixture.start_prior_generation().await;

    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xba),
        files: vec![("src/lib.rs", sha(0xbb), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: Some(SourceMutation {
            db: fixture.db().clone(),
            group_id: fixture.group_id,
            repository_key: fixture.repository_key,
            commit_sha: sha(0xbc),
            action: SourceMutationAction::ConnectionDisable(connection_key.to_string()),
        }),
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await;
    let reads = fixture
        .reads(result.as_ref().ok().map(|outcome| outcome.generation_key))
        .await;
    fixture.cleanup().await;

    let error = result.expect_err("a disabled connection must reject the paused acquisition");
    assert!(
        error.to_string().contains("git_index_stale_target"),
        "{error}"
    );
    assert_eq!(
        reads.active,
        Some(prior),
        "the previous active generation keeps serving"
    );
    assert_ne!(
        reads.source.expect("source still exists").index_status,
        GitIndexStatus::Ready,
        "the stale attempt never marks the newer source ready"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn source_transitions_reject_an_outdated_configuration_revision() {
    let fixture = Fixture::setup().await;
    let source = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read source")
        .expect("the fixture owns the source");
    let stale_revision = source.config_revision;
    let building = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &crate::db::NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: sha(0xad),
                index_profile: GitIndexProfile::Lexical,
            },
        )
        .await
        .expect("start a building generation");
    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &source.include_patterns,
                exclude_patterns: &["docs/**".to_string()],
                interval_seconds: source.interval_seconds,
                webhook_enabled: source.webhook_enabled,
                enabled: source.enabled,
                retention_count: source.retention_count,
            },
        )
        .await
        .expect("edit the source")
        .expect("the fixture owns the source");

    assert!(
        !fixture
            .db()
            .begin_git_repository_index(
                fixture.group_id,
                fixture.repository_key,
                stale_revision,
                source.checkpoint.target_commit_sha.as_deref(),
                &sha(0xad),
            )
            .await
            .expect("begin a stale attempt")
    );
    assert!(
        !fixture
            .db()
            .complete_git_repository_noop(
                fixture.group_id,
                fixture.repository_key,
                stale_revision,
                source.checkpoint.target_commit_sha.as_deref(),
                &sha(0xad),
                2,
                "fingerprint",
                &source.last_skip_summary,
            )
            .await
            .expect("complete a stale no-op")
    );
    assert!(
        !fixture
            .db()
            .fail_git_repository_index(fixture.group_id, fixture.repository_key, stale_revision,)
            .await
            .expect("fail a stale attempt")
    );
    let publication = crate::db::GitGenerationPublication {
        expected_config_revision: stale_revision,
        observed_commit_sha: sha(0xad),
        pipeline_version: 2,
        config_fingerprint: "fingerprint".to_string(),
        skip_summary: source.last_skip_summary,
        coverage: crate::db::GitGenerationCoverage {
            file_count: 0,
            excluded_file_count: 0,
            total_bytes: 0,
        },
        observed_force_rebuild_requested_at: None,
    };
    assert!(
        fixture
            .db()
            .publish_git_repository_generation(
                fixture.group_id,
                fixture.repository_key,
                building.generation_key,
                &publication,
            )
            .await
            .expect("publish a stale attempt")
            .is_none(),
        "a stale publication changes nothing"
    );
    assert!(
        fixture
            .db()
            .get_git_active_generation(fixture.group_id, fixture.repository_key)
            .await
            .expect("read active generation")
            .is_none(),
        "a rejected publication never activates"
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_force_demand_during_a_run_survives_and_forces_the_follow_up() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xaf);
    let first = FakeProvider::new(FakeAcquirerSpec {
        commit: commit.clone(),
        files: vec![("src/lib.rs", sha(0xd6), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: Some(SourceMutation {
            db: fixture.db().clone(),
            group_id: fixture.group_id,
            repository_key: fixture.repository_key,
            commit_sha: commit.clone(),
            action: SourceMutationAction::ForceRebuild,
        }),
    });
    let first_outcome = index_git_repository_snapshot(
        fixture.db(),
        &first,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the run publishes");
    let after_first = fixture.reads(Some(first_outcome.generation_key)).await;
    let demanded_after = after_first
        .source
        .as_ref()
        .expect("source still exists")
        .force_rebuild_requested_at;
    assert!(
        demanded_after.is_some(),
        "a demand arriving during the run survives its publication"
    );

    let follower = FakeProvider::new(FakeAcquirerSpec {
        commit,
        files: vec![("src/lib.rs", sha(0xd6), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let follow_up = index_git_repository_snapshot(
        fixture.db(),
        &follower,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the follow-up rebuilds");
    let reads = fixture.reads(Some(follow_up.generation_key)).await;
    fixture.cleanup().await;

    assert!(!follow_up.no_op, "the durable demand forces a rebuild");
    assert_ne!(follow_up.generation_key, first_outcome.generation_key);
    let source = reads.source.expect("source still exists");
    assert_eq!(
        source.force_rebuild_requested_at, None,
        "the captured demand is consumed after a successful publish"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_snapshot_refuses_a_source_revision_captured_before_provider_construction() {
    let fixture = Fixture::setup().await;
    let prior = fixture.start_prior_generation().await;
    let captured = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read source")
        .expect("the fixture owns the source")
        .config_revision;
    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: false,
                retention_count: 2,
            },
        )
        .await
        .expect("disable the source after the captured revision")
        .expect("the fixture owns the source");

    let provider = FakeProvider::new(binary_spec(&sha(0xb7)));
    let result = index_git_repository_snapshot_at_revision(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
        captured,
    )
    .await;
    let reads = fixture.reads(None).await;
    fixture.cleanup().await;

    let error = result.expect_err("a newer source revision must reject the old provider attempt");
    assert!(
        error.to_string().contains("git_index_stale_target"),
        "{error}"
    );
    assert_eq!(
        reads.active,
        Some(prior),
        "the previous active generation keeps serving"
    );
    assert_eq!(
        reads.generations.len(),
        1,
        "the stale attempt never starts a generation"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_connection_disable_invalidates_the_source_acquisition_revision() {
    let fixture = Fixture::setup().await;
    let connection_key = "revision-connection";
    fixture
        .db()
        .upsert_git_provider_connection(
            fixture.group_id,
            &NewGitProviderConnection {
                connection_key: connection_key.to_string(),
                provider: GitProviderKind::GitHub,
                mode: GitConnectionMode::Public,
                display_name: "Revision connection".to_string(),
                base_url: "https://github.com".to_string(),
                credential_secret_key: None,
                webhook_secret_key: None,
            },
        )
        .await
        .expect("seed the provider connection");
    fixture
        .attach_connection(connection_key)
        .await
        .expect("attach the connection");

    let before = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read the attached source")
        .expect("the fixture owns the source")
        .config_revision;
    assert!(
        fixture
            .db()
            .disable_git_provider_connection(fixture.group_id, connection_key)
            .await
            .expect("disable the connection"),
        "the fixture owns the connection"
    );
    let after = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read the disabled source")
        .expect("the fixture owns the source")
        .config_revision;
    fixture.cleanup().await;

    assert_eq!(
        after,
        before + 1,
        "disabling the connection must invalidate the source acquisition revision"
    );
}
