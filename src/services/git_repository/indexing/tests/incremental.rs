//! Incremental reuse, rebuilds, and forced regeneration against a baseline.

use std::sync::atomic::Ordering;

use crate::contracts::sources::GitIndexProfile;
use crate::db::GitRepositorySettingsUpdate;
use crate::services::git_repository::code_vectors::{
    CodeVectorRequest, code_destination_fingerprint,
};

use super::super::super::{limits::GitIndexLimits, support::*};
use super::super::index_git_repository_snapshot;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_incremental_target_fetches_only_changed_and_added_blobs() {
    let fixture = Fixture::setup().await;
    let keep = sha(0xc1);
    let changed = sha(0xc2);
    let changed_new = sha(0xc3);
    let added = sha(0xc4);
    let first = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xb2),
        files: vec![
            ("src/keep.rs", keep.clone(), b"pub fn keep() {}\n"),
            ("src/change.rs", changed.clone(), b"pub fn old() {}\n"),
            ("src/deleted.rs", sha(0xc5), b"pub fn gone() {}\n"),
        ],
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
    .expect("the first snapshot indexes");

    let second_commit = sha(0xb3);
    let second = FakeProvider::new(FakeAcquirerSpec {
        commit: second_commit.clone(),
        files: vec![
            ("src/keep.rs", keep, b"pub fn keep() {}\n"),
            ("src/change.rs", changed_new, b"pub fn new() {}\n"),
            ("src/added.rs", added, b"pub fn added() {}\n"),
        ],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    fixture.observe_target(&second_commit).await;
    let outcome = index_git_repository_snapshot(
        fixture.db(),
        &second,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the incremental snapshot indexes");
    let fetched = second.fetched.lock().unwrap().clone();
    let reads = fixture.reads(Some(outcome.generation_key)).await;
    fixture.cleanup().await;

    assert!(!outcome.no_op);
    assert!(outcome.generation_key != first_outcome.generation_key);
    assert_eq!(outcome.reused_file_count, 1);
    assert_eq!(outcome.changed_file_count, 2);
    assert_eq!(outcome.file_count, 3);
    assert!(
        !fetched.contains(&changed),
        "the unchanged file's blob is never fetched"
    );
    assert_eq!(
        fetched.len(),
        2,
        "only the changed and added blobs are fetched"
    );
    assert_eq!(
        reads
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/added.rs", "src/change.rs", "src/keep.rs"],
        "the deleted path is absent and the rest survive"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_incremental_hybrid_generation_reuses_unchanged_vectors() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let keep = sha(0xd1);
    let files: Vec<(&'static str, String, &'static [u8])> = vec![
        ("src/keep.rs", keep.clone(), b"pub fn keep() {}\n"),
        ("README.md", sha(0xd2), b"# hello\n"),
    ];
    let first = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xb4),
        files: files.clone(),
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let code_provider = FakeCodeProvider::new(8, false);
    let calls = code_provider.calls.clone();
    let sink = RecordingCodeSink::new(false);
    let request = CodeVectorRequest::with_test_sink(
        code_snapshot(code_provider, 8, "fp-reuse"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let first_outcome = index_git_repository_snapshot(
        fixture.db(),
        &first,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await
    .expect("the first hybrid snapshot indexes");
    let initial_calls = calls.load(Ordering::SeqCst);
    let initial_points = sink.point_count();

    let second_commit = sha(0xb5);
    let second = FakeProvider::new(FakeAcquirerSpec {
        commit: second_commit.clone(),
        files,
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    fixture.observe_target(&second_commit).await;
    let outcome = index_git_repository_snapshot(
        fixture.db(),
        &second,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await
    .expect("the incremental hybrid snapshot indexes");
    let final_calls = calls.load(Ordering::SeqCst);
    let all_points = sink.points.lock().unwrap().clone();
    let reads = fixture.reads(Some(outcome.generation_key)).await;
    fixture.cleanup().await;

    assert!(outcome.generation_key != first_outcome.generation_key);
    assert_eq!(outcome.reused_file_count, 2);
    assert_eq!(
        final_calls, initial_calls,
        "unchanged chunks are not re-embedded"
    );
    assert_eq!(
        sink.point_count(),
        initial_points * 2,
        "every target chunk receives a point"
    );
    assert!(
        all_points[initial_points..]
            .iter()
            .all(|point| point.generation_key == outcome.generation_key),
        "reused vectors carry the new generation identity"
    );
    let metadata = reads.code_vectors.expect("code destination recorded");
    assert_eq!(metadata.point_count, 2);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_incremental_generation_embeds_chunks_whose_baseline_point_is_missing() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let files: Vec<(&'static str, String, &'static [u8])> = vec![
        ("src/keep.rs", sha(0xe1), b"pub fn keep() {}\n"),
        ("README.md", sha(0xe2), b"# hello\n"),
    ];
    let first = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xb6),
        files: files.clone(),
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let code_provider = FakeCodeProvider::new(8, false);
    let calls = code_provider.calls.clone();
    let sink = RecordingCodeSink::new(false);
    let request = CodeVectorRequest::with_test_sink(
        code_snapshot(code_provider, 8, "fp-missing"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let first_outcome = index_git_repository_snapshot(
        fixture.db(),
        &first,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await
    .expect("the first hybrid snapshot indexes");
    let initial_calls = calls.load(Ordering::SeqCst);
    let initial_points = sink.point_count();
    sink.forget_vectors();

    let second_commit = sha(0xb7);
    let second = FakeProvider::new(FakeAcquirerSpec {
        commit: second_commit.clone(),
        files,
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    fixture.observe_target(&second_commit).await;
    let outcome = index_git_repository_snapshot(
        fixture.db(),
        &second,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await
    .expect("the incremental snapshot falls back to embedding");

    assert_ne!(outcome.generation_key, first_outcome.generation_key);
    assert_eq!(outcome.reused_file_count, 2);
    assert_eq!(sink.point_count(), initial_points * 2);
    assert!(
        calls.load(Ordering::SeqCst) > initial_calls,
        "chunks without a retrievable baseline point are embedded again"
    );
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_code_model_change_reembeds_instead_of_reusing_vectors() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let files: Vec<(&'static str, String, &'static [u8])> = vec![
        ("src/keep.rs", sha(0xe3), b"pub fn keep() {}\n"),
        ("README.md", sha(0xe4), b"# hello\n"),
    ];
    let commit = sha(0xb8);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: commit.clone(),
        files,
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let code_provider = FakeCodeProvider::new(8, false);
    let calls = code_provider.calls.clone();
    let sink = RecordingCodeSink::new(false);
    let first_request = CodeVectorRequest::with_test_sink(
        code_snapshot(code_provider.clone(), 8, "fp-model-a"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let first_outcome = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&first_request),
        false,
    )
    .await
    .expect("the first hybrid snapshot indexes");
    let initial_calls = calls.load(Ordering::SeqCst);
    let second_request = CodeVectorRequest::with_test_sink(
        code_snapshot(code_provider, 8, "fp-model-b"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let outcome = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&second_request),
        false,
    )
    .await
    .expect("the model change rebuilds the generation");

    assert_ne!(outcome.generation_key, first_outcome.generation_key);
    assert!(
        calls.load(Ordering::SeqCst) > initial_calls,
        "a changed code model never reuses the old destination's vectors"
    );
    let reads = fixture.reads(Some(outcome.generation_key)).await;
    fixture.cleanup().await;
    let metadata = reads.code_vectors.expect("code destination recorded");
    assert_eq!(
        metadata.fingerprint,
        code_destination_fingerprint("fp-model-b", "http://qdrant.invalid:6334", 8)
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_config_change_rebuilds_instead_of_reusing_the_baseline() {
    let fixture = Fixture::setup().await;
    let files: Vec<(&'static str, String, &'static [u8])> = vec![
        ("src/keep.rs", sha(0xe5), b"pub fn keep() {}\n"),
        ("README.md", sha(0xe6), b"# hello\n"),
    ];
    let first = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xb9),
        files: files.clone(),
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
    .expect("the first snapshot indexes");
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
                exclude_patterns: &["docs/**".to_string()],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 2,
            },
        )
        .await
        .expect("update settings")
        .expect("the fixture owns the source");

    let second_commit = sha(0xba);
    let second = FakeProvider::new(FakeAcquirerSpec {
        commit: second_commit.clone(),
        files,
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    fixture.observe_target(&second_commit).await;
    let outcome = index_git_repository_snapshot(
        fixture.db(),
        &second,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the config change rebuilds");

    assert_ne!(outcome.generation_key, first_outcome.generation_key);
    assert_eq!(outcome.reused_file_count, 0);
    assert_eq!(outcome.changed_file_count, 2);
    assert_eq!(second.fetched.lock().unwrap().len(), 2);
    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_force_rebuild_recreates_an_unchanged_target_while_sync_noops() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xae);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: commit.clone(),
        files: vec![("src/lib.rs", sha(0xd5), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let first = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("the first snapshot indexes");
    let synced = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("a normal sync no-ops");
    let rebuilt = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        true,
    )
    .await
    .expect("a force rebuild writes a fresh snapshot");

    let reads = fixture.reads(Some(rebuilt.generation_key)).await;
    fixture.cleanup().await;

    assert!(synced.no_op);
    assert_eq!(synced.generation_key, first.generation_key);
    assert!(!rebuilt.no_op);
    assert_ne!(rebuilt.generation_key, first.generation_key);
    assert_eq!(rebuilt.commit_sha, commit);
    assert_eq!(rebuilt.reused_file_count, 0);
    assert_eq!(rebuilt.changed_file_count, 1);
    assert_eq!(reads.active, Some(rebuilt.generation_key));
}
