//! Bounded snapshot indexing tests (issue #681 work unit 3B3).
//!
//! The orchestration runs against the real group-scoped repository,
//! generation, and content methods with a scripted in-memory acquirer, so no
//! live provider is contacted. Cases that need persistence connect through the
//! crate's standard test constructor to the already-migrated scratch database
//! named by `CONTEXT69_TEST_DATABASE_URL` (skipped when it is unset), author no
//! migration, and delete their own group so no rows are left behind. Each test
//! reads its state before cleanup, so a failing assertion leaves no residue.
//! The doubles and fixture live in [`super::super::limits::support`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::code_embedding_runtime::CodeEmbeddingRuntime;
use crate::contracts::sources::{
    GitConnectionMode, GitGenerationStatus, GitIndexProfile, GitIndexStatus, GitProviderKind,
};
use crate::db::{GitCheckpointUpdate, GitRepositorySettingsUpdate, NewGitProviderConnection};
use crate::embedding::EmbeddingProvider;
use crate::qdrant_index::code_vectors::CodeVectorPoint;

use super::super::GitAcquisitionLimits;
use super::super::limits::GitIndexLimits;
use super::super::support::*;
use super::{
    ForgejoSnapshotProvider, GitHubSnapshotProvider, GitSnapshotProvider,
    index_git_repository_snapshot, index_git_repository_snapshot_at_revision,
};
use crate::services::git_repository::ForgejoOrigin;
use crate::services::git_repository::code_vectors::{
    CodeVectorRequest, CodeVectorSink, code_destination_fingerprint, code_vector_target,
};

#[test]
fn the_public_provider_builds_a_credential_free_acquirer() {
    let provider = GitHubSnapshotProvider::new(Arc::new(NeverTransport));
    assert!(
        provider
            .acquirer(
                "https://github.com/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_ok(),
        "public acquisition must not need a credential"
    );
    assert!(
        provider
            .acquirer(
                "https://evil.invalid/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_err(),
        "the factory must reject a non-canonical GitHub URL"
    );
}

#[test]
fn the_forgejo_provider_binds_the_connection_origin() {
    let provider = ForgejoSnapshotProvider::new(
        Arc::new(NeverTransport),
        ForgejoOrigin::from_base_url("https://forgejo.example/api/v1").expect("origin"),
    );
    assert!(
        provider
            .acquirer(
                "https://forgejo.example/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_ok(),
        "public Forgejo acquisition needs no credential"
    );
    assert!(
        provider
            .acquirer(
                "https://evil.example/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_err(),
        "a repository outside the connection origin must be refused"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_bounded_snapshot_persists_exact_content_and_activates() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xa1);
    let provider = FakeProvider::new(binary_spec(&commit));
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
    let created = provider.created.load(std::sync::atomic::Ordering::SeqCst);
    let seen_url = provider.seen_url.lock().unwrap().clone();
    fixture.cleanup().await;

    let outcome = result.expect("the snapshot must index");
    assert_eq!(outcome.commit_sha, commit);
    assert_eq!(outcome.generation_number, 1);
    assert_eq!(outcome.file_count, 2);
    assert_eq!(
        outcome.excluded_file_count, 3,
        "two listing exclusions plus the binary file"
    );
    assert_eq!(reads.active, Some(outcome.generation_key));
    assert_eq!(
        reads
            .generations
            .first()
            .map(|generation| generation.status),
        Some(GitGenerationStatus::Ready)
    );
    assert_eq!(
        reads
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["README.md", "src/main.rs"]
    );
    let main = reads
        .files
        .iter()
        .find(|file| file.path == "src/main.rs")
        .expect("src/main.rs must be in the manifest");
    assert_eq!(main.language, "rust");
    assert_eq!(main.byte_count, 13);
    assert_eq!(reads.chunks.len(), 1);
    assert_eq!(reads.chunks[0].text, "fn main() {}\n");
    assert_eq!(
        (reads.chunks[0].start_line, reads.chunks[0].end_line),
        (1, 1)
    );
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(
        source.checkpoint.target_commit_sha.as_deref(),
        Some(commit.as_str())
    );
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(commit.as_str())
    );
    assert_eq!(source.active_generation_key, Some(outcome.generation_key));
    assert_eq!(created, 1, "one fresh acquirer per generation");
    assert_eq!(seen_url.as_deref(), Some(source.canonical_url.as_str()));
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_empty_eligible_snapshot_completes_safely() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xa2);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: commit.clone(),
        files: Vec::new(),
        excluded: 5,
        too_large: 0,
        fail_blob: None,
        mutate: None,
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

    let outcome = result.expect("an empty snapshot still completes");
    assert_eq!(outcome.file_count, 0);
    assert_eq!(outcome.excluded_file_count, 5);
    assert!(reads.files.is_empty());
    assert_eq!(reads.active, Some(outcome.generation_key));
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(commit.as_str())
    );
}

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
async fn a_failed_fetch_leaves_the_previous_active_generation_serving() {
    let fixture = Fixture::setup().await;
    let prior = fixture.start_prior_generation().await;
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa4),
        files: vec![("src/lib.rs", sha(0xc2), b"pub fn x() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: Some(sha(0xc2)),
        mutate: None,
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

    let error = result.expect_err("a fetch failure must fail the attempt");
    assert!(error.to_string().contains("git_http_status_500"), "{error}");
    assert_eq!(
        reads.active,
        Some(prior),
        "the previous active generation keeps serving"
    );
    let status = |key: Uuid| {
        reads
            .generations
            .iter()
            .find(|generation| generation.generation_key == key)
            .map(|generation| generation.status)
    };
    assert_eq!(status(prior), Some(GitGenerationStatus::Ready));
    let failed = reads
        .generations
        .iter()
        .find(|generation| generation.generation_key != prior)
        .expect("a new generation row must exist")
        .generation_key;
    assert_eq!(status(failed), Some(GitGenerationStatus::Failed));
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Failed);
    assert_eq!(source.active_generation_key, Some(prior));
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lexical_source_never_invokes_the_code_sink() {
    let fixture = Fixture::setup().await;
    let provider = FakeProvider::new(binary_spec(&sha(0xe1)));
    let code_provider = FakeCodeProvider::new(8, false);
    let calls = code_provider.calls.clone();
    let sink = RecordingCodeSink::new(false);
    let request = CodeVectorRequest::with_test_sink(
        code_snapshot(code_provider, 8, "fp-lexical"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    fixture.cleanup().await;

    result.expect("a lexical snapshot still activates");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a lexical source must not embed code"
    );
    assert!(
        reads.code_vectors.is_none(),
        "a lexical generation records no code metadata"
    );
    assert_eq!(sink.write_count(), 0);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_hybrid_source_acknowledges_every_chunk_and_records_its_destination() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let provider = FakeProvider::new(binary_spec(&sha(0xe2)));
    let sink = RecordingCodeSink::new(false);
    let request = CodeVectorRequest::with_test_sink(
        code_snapshot(FakeCodeProvider::new(8, false), 8, "fp-hybrid"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    fixture.cleanup().await;

    let outcome = result.expect("a hybrid snapshot must index");
    assert_eq!(outcome.file_count, 2);
    assert_eq!(
        sink.point_count(),
        2,
        "one point per stored chunk of the two eligible files"
    );
    let paths = sink
        .points
        .lock()
        .unwrap()
        .iter()
        .map(|point| point.path.clone())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"src/main.rs".to_string()));
    assert!(paths.contains(&"README.md".to_string()));

    let metadata = reads
        .code_vectors
        .expect("the activated generation records its code destination");
    assert_eq!(metadata.point_count, 2);
    assert_eq!(metadata.dimensions, 8);
    assert_eq!(
        metadata.fingerprint,
        code_destination_fingerprint("fp-hybrid", "http://qdrant.invalid:6334", 8)
    );
    assert_eq!(
        metadata.collection_name,
        format!("context69_code_v1_{}", metadata.fingerprint)
    );
    assert_eq!(reads.active, Some(outcome.generation_key));
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failing_code_sink_keeps_the_previous_active_generation() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let prior = fixture.start_prior_generation().await;
    let provider = FakeProvider::new(binary_spec(&sha(0xe3)));
    let request = CodeVectorRequest::with_test_sink(
        code_snapshot(FakeCodeProvider::new(8, false), 8, "fp-failing"),
        Some("http://qdrant.invalid:6334".to_string()),
        RecordingCodeSink::new(true),
    );
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    let failed = reads
        .generations
        .iter()
        .find(|generation| generation.generation_key != prior)
        .expect("a new generation row must exist")
        .clone();
    let retained = fixture
        .db()
        .get_git_generation_code_vectors(
            fixture.group_id,
            fixture.repository_key,
            failed.generation_key,
        )
        .await
        .expect("read the retained destination");
    fixture.cleanup().await;

    let error = result.expect_err("a failed sink must fail the generation");
    assert!(
        error.to_string().contains("git_code_sink_failed"),
        "{error}"
    );
    assert_eq!(
        reads.active,
        Some(prior),
        "a partial code pass never activates over the previous generation"
    );
    assert!(
        retained.is_some_and(|metadata| metadata.point_count == 0),
        "a failed code pass retains the provisional destination, so remote points stay owned"
    );
    assert_eq!(failed.status, GitGenerationStatus::Failed);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_partial_vector_write_keeps_ownership_until_cleanup_deletes_it() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let provider = FakeProvider::new(binary_spec(&sha(0xe4)));
    let sink = Arc::new(FailAfterFirstWriteSink::new());
    let request = CodeVectorRequest::with_test_sink(
        code_snapshot(FakeCodeProvider::new(8, false), 8, "fp-partial"),
        Some("http://qdrant.invalid:6334".to_string()),
        sink,
    );
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;

    let error = result.expect_err("a partial sink failure must fail the generation");
    assert!(
        error
            .to_string()
            .contains("git_code_sink_failed_after_partial"),
        "{error}"
    );
    assert!(
        reads.active.is_none(),
        "nothing activates on a partial pass"
    );
    let failed = reads
        .generations
        .first()
        .expect("the failed generation row exists")
        .clone();
    let metadata = fixture
        .db()
        .get_git_generation_code_vectors(
            fixture.group_id,
            fixture.repository_key,
            failed.generation_key,
        )
        .await
        .expect("read the provisional destination")
        .expect("the provisional destination survives the partial write");
    assert_eq!(metadata.point_count, 0, "the provisional count is zero");
    assert_eq!(metadata.dimensions, 8);
    assert_eq!(failed.status, GitGenerationStatus::Failed);
    let cleanup_pending: bool = sqlx::query_scalar(
        "SELECT cleanup_pending FROM context69.git_repository_generations \
         WHERE generation_key = $1",
    )
    .bind(failed.generation_key)
    .fetch_one(fixture.db().pool())
    .await
    .expect("read the cleanup ownership flag");
    assert!(
        cleanup_pending,
        "a partial remote write leaves independent cleanup owning the generation"
    );

    sqlx::query(
        "UPDATE context69.git_repository_generations \
         SET cleanup_next_retry_at = now() - interval '1 second' \
         WHERE generation_key = $1",
    )
    .bind(failed.generation_key)
    .execute(fixture.db().pool())
    .await
    .expect("make the pending cleanup due");
    let pending = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list pending cleanup sources");
    assert!(
        pending
            .iter()
            .any(|(_, repository_key, _)| *repository_key == fixture.repository_key),
        "the partial write is retried without another index"
    );
    super::super::refresh::run_pending_generation_cleanup(
        fixture.db(),
        pending,
        |_group_id, _repository_key, _generation_key, _metadata| async move {
            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .expect("the independent cleanup pass completes the vector deletion");
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations after cleanup");
    assert!(
        !generations
            .iter()
            .any(|generation| generation.generation_key == failed.generation_key),
        "the failed generation is removed once its vectors are gone"
    );
    assert!(
        fixture
            .db()
            .get_git_generation_code_vectors(
                fixture.group_id,
                fixture.repository_key,
                failed.generation_key,
            )
            .await
            .expect("read code vectors after cleanup")
            .is_none(),
        "the only remote-ownership record is gone with the generation"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_hybrid_source_without_a_code_runtime_stays_lexical_without_metadata() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let provider = FakeProvider::new(binary_spec(&sha(0xe4)));
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

    let outcome = result.expect("an absent code runtime still activates lexically");
    assert_eq!(reads.active, Some(outcome.generation_key));
    assert!(reads.code_vectors.is_none());
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_hybrid_source_with_a_configured_code_model_and_no_qdrant_url_fails() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let provider = FakeProvider::new(binary_spec(&sha(0xe5)));
    let request = CodeVectorRequest::production(
        code_snapshot(FakeCodeProvider::new(8, false), 8, "fp-no-url"),
        None,
    );
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    fixture.cleanup().await;

    let error = result.expect_err("a missing endpoint must not downgrade silently");
    assert!(
        error.to_string().contains("git_code_qdrant_url_missing"),
        "{error}"
    );
    assert!(reads.active.is_none());
    assert!(reads.code_vectors.is_none());
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_same_target_reuses_the_active_generation_without_writing_one() {
    let fixture = Fixture::setup().await;
    let commit = sha(0xb1);
    let provider = FakeProvider::new(binary_spec(&commit));
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
    let second = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("an unchanged target is a no-op");
    let reads = fixture.reads(None).await;
    fixture.cleanup().await;

    assert!(second.no_op);
    assert_eq!(second.generation_key, first.generation_key);
    assert_eq!(
        reads.generations.len(),
        1,
        "no second generation is written"
    );
    assert_eq!(reads.active, Some(first.generation_key));
    let source = reads.source.expect("source still exists");
    assert_eq!(source.index_status, GitIndexStatus::Ready);
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(commit.as_str())
    );
}

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
async fn an_explicit_pin_never_drifts_when_the_branch_advances() {
    let fixture = Fixture::setup().await;
    let pinned = sha(0xa8);
    let source = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read source")
        .expect("the fixture owns the source");
    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "set",
                pinned_commit_value: Some(&pinned),
                index_profile: None,
                include_patterns: &source.include_patterns,
                exclude_patterns: &source.exclude_patterns,
                interval_seconds: source.interval_seconds,
                webhook_enabled: source.webhook_enabled,
                enabled: true,
                retention_count: source.retention_count,
            },
        )
        .await
        .expect("pin the source")
        .expect("the fixture owns the source");

    let advancing = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa9),
        files: vec![("src/one.rs", sha(0xd3), b"pub fn one() {}\n")],
        excluded: 0,
        too_large: 0,
        fail_blob: None,
        mutate: None,
    });
    let first = index_git_repository_snapshot(
        fixture.db(),
        &advancing,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("a pinned source indexes the configured commit");
    let second = index_git_repository_snapshot(
        fixture.db(),
        &advancing,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        None,
        false,
    )
    .await
    .expect("a pinned source stays on the configured commit");

    let reads = fixture.reads(Some(first.generation_key)).await;
    fixture.cleanup().await;

    assert_eq!(first.commit_sha, pinned);
    assert!(second.no_op);
    assert_eq!(second.commit_sha, pinned);
    let source = reads.source.expect("source still exists");
    assert_eq!(source.version.commit_sha.as_deref(), Some(pinned.as_str()));
    assert_eq!(
        source.checkpoint.indexed_commit_sha.as_deref(),
        Some(pinned.as_str())
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
    sqlx::query(
        "UPDATE context69.git_repository_sources SET connection_key = $1 \
         WHERE group_id = $2 AND repository_key = $3",
    )
    .bind(connection_key)
    .bind(fixture.group_id)
    .bind(fixture.repository_key)
    .execute(fixture.db().pool())
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

struct PausingProvider {
    entered: Notify,
    resume: Notify,
    first_call: AtomicBool,
    calls: AtomicUsize,
}

impl PausingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            resume: Notify::new(),
            first_call: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for PausingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.first_call.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        Ok(texts.iter().map(|_| vec![0.25_f32; 8]).collect())
    }
}

struct FailAfterFirstWriteSink {
    writes: AtomicUsize,
}

impl FailAfterFirstWriteSink {
    fn new() -> Self {
        Self {
            writes: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl CodeVectorSink for FailAfterFirstWriteSink {
    async fn write(&self, _dimensions: usize, _points: &[CodeVectorPoint]) -> Result<()> {
        if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(());
        }
        Err(anyhow::anyhow!("git_code_sink_failed_after_partial"))
    }
}

struct PausingSink {
    entered: Notify,
    resume: Notify,
    first_write: AtomicBool,
    writes: AtomicUsize,
    points: std::sync::Mutex<Vec<CodeVectorPoint>>,
}

impl PausingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            resume: Notify::new(),
            first_write: AtomicBool::new(true),
            writes: AtomicUsize::new(0),
            points: std::sync::Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl CodeVectorSink for PausingSink {
    async fn write(&self, _dimensions: usize, points: &[CodeVectorPoint]) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.first_write.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.points.lock().unwrap().extend(points.iter().cloned());
        Ok(())
    }
}

/// A runtime change while the code provider is paused keeps the captured
/// destination and the old provider's vectors; a later request derives the new
/// identity.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_runtime_change_during_embedding_keeps_the_captured_destination() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let provider = FakeProvider::new(binary_spec(&sha(0xf1)));
    let runtime = CodeEmbeddingRuntime::new();
    let pausing = PausingProvider::new();
    let old_config = code_snapshot(pausing.clone(), 8, "fp-old").config.clone();
    runtime.install(pausing.clone(), old_config.clone(), "fp-old".to_string());

    let url = "http://qdrant.invalid:6334".to_string();
    let old_snapshot = runtime
        .snapshot()
        .expect("the old code runtime is installed");
    let old_target = code_vector_target(&CodeVectorRequest::production(
        old_snapshot.clone(),
        Some(url.clone()),
    ))
    .expect("the old destination resolves");
    let sink = RecordingCodeSink::new(false);
    let request = CodeVectorRequest::with_test_sink(old_snapshot, Some(url.clone()), sink.clone());

    let run = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    );
    let mutate = async {
        pausing.entered.notified().await;
        runtime.install(
            FakeCodeProvider::new(8, false),
            old_config,
            "fp-new".to_string(),
        );
        pausing.resume.notify_one();
    };
    let (result, ()) = tokio::join!(run, mutate);

    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    let points = sink.points.lock().unwrap().clone();
    let calls = pausing.calls.load(Ordering::SeqCst);
    fixture.cleanup().await;

    let outcome = result.expect("the generation must still index");
    let metadata = reads
        .code_vectors
        .expect("the captured destination is recorded");
    assert_eq!(metadata.fingerprint, old_target.destination_fingerprint());
    assert_eq!(metadata.collection_name, old_target.collection_name());
    assert_eq!(metadata.dimensions, 8);
    assert!(!points.is_empty(), "the old provider acknowledged points");
    assert!(
        points
            .iter()
            .all(|point| point.vector.iter().all(|value| *value == 0.25)),
        "every point came from the captured old provider, never the new one"
    );
    assert!(calls >= 1, "the old provider was actually used");

    let new_snapshot = runtime
        .snapshot()
        .expect("the new code runtime is installed");
    let new_target = code_vector_target(&CodeVectorRequest::production(new_snapshot, Some(url)))
        .expect("the new destination resolves");
    assert_ne!(
        new_target.destination_fingerprint(),
        old_target.destination_fingerprint(),
        "a later target derivation sees the new identity"
    );
    assert_eq!(reads.active, Some(outcome.generation_key));
}

/// A source target change while the code sink is paused invalidates the
/// attempt after the vector pass; it must neither activate nor advance the
/// checkpoint, and the previous ready generation keeps serving.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_target_change_during_the_vector_pass_never_activates() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let prior = fixture.start_prior_generation().await;
    let provider = FakeProvider::new(binary_spec(&sha(0xf2)));
    let snapshot = code_snapshot(FakeCodeProvider::new(8, false), 8, "fp-stale");
    let sink = PausingSink::new();
    let request = CodeVectorRequest::with_test_sink(
        snapshot,
        Some("http://qdrant.invalid:6334".to_string()),
        sink.clone(),
    );
    let changed = sha(0x99);

    let run = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
        Some(&request),
        false,
    );
    let mutate = async {
        sink.entered.notified().await;
        fixture
            .db()
            .update_git_repository_checkpoint(
                fixture.group_id,
                fixture.repository_key,
                &GitCheckpointUpdate {
                    target_commit_sha: Some(changed.clone()),
                    indexed_commit_sha: None,
                    index_status: GitIndexStatus::Indexing,
                    pipeline_version: None,
                    config_fingerprint: None,
                    skip_summary: None,
                },
            )
            .await
            .expect("move the target mid-pass")
            .expect("the fixture owns the source");
        sink.resume.notify_one();
    };
    let (result, ()) = tokio::join!(run, mutate);

    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
    let writes = sink.writes.load(Ordering::SeqCst);
    let failed = reads
        .generations
        .iter()
        .find(|generation| generation.generation_key != prior)
        .expect("the failed generation row exists")
        .clone();
    let retained = fixture
        .db()
        .get_git_generation_code_vectors(
            fixture.group_id,
            fixture.repository_key,
            failed.generation_key,
        )
        .await
        .expect("read the retained destination");
    fixture.cleanup().await;

    let error = result.expect_err("a changed target must reject the attempt");
    assert!(
        error.to_string().contains("git_index_stale_target"),
        "{error}"
    );
    assert_eq!(
        reads.active,
        Some(prior),
        "the previous ready generation keeps serving"
    );
    assert!(
        retained.is_some_and(|metadata| metadata.point_count == 0),
        "a stale attempt retains the provisional destination, so written points stay owned"
    );
    assert_eq!(failed.status, GitGenerationStatus::Failed);
    let source = reads.source.expect("the source still exists");
    assert_eq!(
        source.checkpoint.target_commit_sha.as_deref(),
        Some(changed.as_str())
    );
    assert!(
        source.checkpoint.indexed_commit_sha.is_none(),
        "the stale attempt never advances the indexed checkpoint"
    );
    assert_eq!(source.index_status, GitIndexStatus::Indexing);
    assert!(writes >= 1, "the sink ran before the target changed");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_source_before_its_first_generation_lists_a_null_active_pointer() {
    let fixture = Fixture::setup().await;
    let repository_key = fixture.repository_key;
    let listed = fixture
        .db()
        .list_git_repository_sources(fixture.group_id)
        .await;
    let fetched = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await;
    fixture.cleanup().await;

    let listed = listed.expect("list the group's sources");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].repository_key, repository_key);
    assert!(
        listed[0].active_generation_key.is_none(),
        "a source with no active generation must read a null pointer, not depend on planner inference"
    );
    let fetched = fetched
        .expect("read the source")
        .expect("the fixture owns the source");
    assert!(fetched.active_generation_key.is_none());
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lexical_generation_enumerates_without_a_destination_and_deletes_with_its_source() {
    let fixture = Fixture::setup().await;
    let generation_key = fixture.start_prior_generation().await;
    let destinations = fixture
        .db()
        .list_git_generations_with_code_vectors(fixture.group_id, fixture.repository_key)
        .await;
    let deleted = fixture
        .db()
        .delete_git_repository_source(fixture.group_id, fixture.repository_key)
        .await;
    let fetched = fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await;
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await;
    fixture.cleanup().await;

    let destinations = destinations.expect("enumerate the source's destinations");
    assert_eq!(destinations.len(), 1);
    assert_eq!(destinations[0].generation_key, generation_key);
    assert!(
        destinations[0].code_vectors.is_none(),
        "a lexical generation has no destination to clean"
    );
    assert!(
        deleted.expect("delete the source"),
        "a lexical-only source still owns its row"
    );
    assert!(
        fetched.expect("read the deleted source").is_none(),
        "the source and its cascade are gone"
    );
    assert!(
        generations
            .expect("read the deleted generations")
            .is_empty(),
        "no generation outlives its source"
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
    sqlx::query(
        "UPDATE context69.git_repository_sources SET connection_key = $1 \
         WHERE group_id = $2 AND repository_key = $3",
    )
    .bind(connection_key)
    .bind(fixture.group_id)
    .bind(fixture.repository_key)
    .execute(fixture.db().pool())
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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn acquisition_too_large_files_reach_the_source_skip_summary() {
    let fixture = Fixture::setup().await;
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xb8),
        files: vec![("src/main.rs", sha(0xb9), b"fn main() {}\n")],
        excluded: 1,
        too_large: 1,
        fail_blob: None,
        mutate: None,
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

    let outcome = result.expect("the bounded snapshot still activates");
    assert_eq!(
        outcome.excluded_file_count, 1,
        "the provider-excluded file stays counted as excluded"
    );
    assert_eq!(
        outcome.skip_summary.too_large, 1,
        "the provider's oversized reason is not discarded"
    );
    let source = reads.source.expect("source still exists");
    assert_eq!(
        source.last_skip_summary.too_large, 1,
        "the persisted source summary carries the oversized reason"
    );
}
