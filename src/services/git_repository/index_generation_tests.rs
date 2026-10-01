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

use uuid::Uuid;

use crate::contracts::sources::{GitGenerationStatus, GitIndexStatus};

use super::super::GitAcquisitionLimits;
use super::super::limits::GitIndexLimits;
use super::super::support::*;
use super::{GitHubSnapshotProvider, GitSnapshotProvider, index_git_repository_snapshot};

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

#[tokio::test]
async fn a_bounded_snapshot_persists_exact_content_and_activates() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let commit = sha(0xa1);
    let provider = FakeProvider::new(binary_spec(&commit));
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
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
async fn an_empty_eligible_snapshot_completes_safely() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let commit = sha(0xa2);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: commit.clone(),
        files: Vec::new(),
        excluded: 5,
        fail_blob: None,
        mutate: None,
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
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
async fn a_changed_target_is_rejected_as_stale() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let concurrent = sha(0xee);
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa3),
        files: vec![("src/lib.rs", sha(0xc1), b"pub fn x() {}\n")],
        excluded: 0,
        fail_blob: None,
        mutate: Some(SourceMutation {
            db: fixture.db().clone(),
            group_id: fixture.group_id,
            repository_key: fixture.repository_key,
            commit_sha: concurrent.clone(),
        }),
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
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
async fn a_failed_fetch_leaves_the_previous_active_generation_serving() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let prior = fixture.start_prior_generation().await;
    let provider = FakeProvider::new(FakeAcquirerSpec {
        commit: sha(0xa4),
        files: vec![("src/lib.rs", sha(0xc2), b"pub fn x() {}\n")],
        excluded: 0,
        fail_blob: Some(sha(0xc2)),
        mutate: None,
    });
    let result = index_git_repository_snapshot(
        fixture.db(),
        &provider,
        GitIndexLimits::default(),
        fixture.group_id,
        fixture.repository_key,
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
