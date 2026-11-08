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
use crate::contracts::sources::{GitGenerationStatus, GitIndexProfile, GitIndexStatus};
use crate::db::GitCheckpointUpdate;
use crate::embedding::EmbeddingProvider;
use crate::qdrant_index::code_vectors::CodeVectorPoint;

use super::super::GitAcquisitionLimits;
use super::super::limits::GitIndexLimits;
use super::super::support::*;
use super::{GitHubSnapshotProvider, GitSnapshotProvider, index_git_repository_snapshot};
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
        None,
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
    )
    .await;
    let key = result.as_ref().ok().map(|outcome| outcome.generation_key);
    let reads = fixture.reads(key).await;
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
        reads.code_vectors.is_none(),
        "a failed code pass records no metadata"
    );
    let failed = reads
        .generations
        .iter()
        .find(|generation| generation.generation_key != prior)
        .expect("a new generation row must exist");
    assert_eq!(failed.status, GitGenerationStatus::Failed);
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
        reads.code_vectors.is_none(),
        "a stale attempt records no code destination"
    );
    let failed = reads
        .generations
        .iter()
        .find(|generation| generation.generation_key != prior)
        .expect("the failed generation row exists");
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
