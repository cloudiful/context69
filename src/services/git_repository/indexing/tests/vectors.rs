//! Code vector destinations, partial writes, runtime changes, and target
//! changes during embedding.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::code_embedding_runtime::CodeEmbeddingRuntime;
use crate::contracts::sources::{GitGenerationStatus, GitIndexProfile, GitIndexStatus};
use crate::db::GitCheckpointUpdate;
use crate::services::git_repository::code_vectors::{
    CodeVectorRequest, code_destination_fingerprint, code_vector_target,
};

use super::super::super::{limits::GitIndexLimits, support::*};
use super::super::index_git_repository_snapshot;
use super::support::{FailAfterFirstWriteSink, PausingProvider, PausingSink};

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
    crate::services::git_repository::refresh::run_pending_generation_cleanup(
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
