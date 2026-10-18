//! Generation and pairing invariants for the shared vector runtime.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::embedding::EmbeddingIdentity;
use crate::qdrant_index::QdrantIndex;

use super::{
    VectorFailureClass, VectorRuntime, classify_failure, test_support::TestEmbeddingProvider,
};

fn identity(model: &str) -> EmbeddingIdentity {
    EmbeddingIdentity {
        base_url: "https://embeddings.example/v1".to_string(),
        model: model.to_string(),
        dimensions: 8,
    }
}

fn snapshot(generation: u64, model: &str) -> super::VectorSnapshot {
    super::VectorSnapshot {
        embedding: Arc::new(TestEmbeddingProvider),
        identity: Some(identity(model)),
        index: Arc::new(QdrantIndex::for_test("test", 8)),
        collection: "test".to_string(),
        generation,
    }
}

#[test]
fn a_stale_publish_is_rejected_and_keeps_the_current_generation() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(true)));
    let first = runtime.generation();
    assert!(runtime.publish(first, snapshot(first, "model-a")));
    assert!(runtime.is_ready());

    let second = runtime.begin_generation();
    assert_ne!(first, second);
    assert!(!runtime.is_ready());

    // The attempt that started under the first generation is stale.
    assert!(!runtime.publish(first, snapshot(first, "model-b")));
    let current = runtime.snapshot().expect("the old snapshot is retained");
    assert_eq!(current.generation, first);
    assert_eq!(
        current
            .identity
            .as_ref()
            .map(|identity| identity.model.as_str()),
        Some("model-a")
    );

    assert!(runtime.publish(second, snapshot(second, "model-c")));
    assert!(runtime.is_ready());
}

#[test]
fn a_stale_failure_cannot_clear_a_newer_generation() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(true)));
    let first = runtime.generation();
    assert!(runtime.publish_failure(first, VectorFailureClass::Transient, "connection refused"));

    let second = runtime.begin_generation();
    assert!(!runtime.publish_failure(first, VectorFailureClass::Transient, "stale"));
    assert!(runtime.failure().is_none());

    assert!(runtime.publish_failure(
        second,
        VectorFailureClass::Configuration,
        "dimension mismatch"
    ));
    assert_eq!(
        runtime.failure().map(|failure| failure.generation),
        Some(second)
    );
}

#[test]
fn a_rebuild_fence_keeps_the_pair_unready_until_its_own_generation_completes() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(true)));
    let base = runtime.generation();
    runtime.publish(base, snapshot(base, "model-a"));

    let rebuild = runtime.begin_rebuild();
    assert!(runtime.is_rebuilding());
    assert!(!runtime.is_ready());
    // Publishing the matching candidate during a rebuild keeps readiness off.
    assert!(runtime.publish(rebuild, snapshot(rebuild, "model-a")));
    assert!(!runtime.is_ready());

    // A settings change supersedes the rebuild.
    let newer = runtime.begin_generation();
    assert!(!runtime.complete_rebuild(rebuild));
    assert!(!runtime.is_ready());
    assert!(!runtime.is_rebuilding());
    assert_eq!(runtime.generation(), newer);
}

#[test]
fn replacing_provider_rejects_an_identity_change() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    let before = runtime.snapshot().expect("published snapshot");
    let index_ptr = Arc::as_ptr(&before.index);

    assert!(
        !runtime.replace_provider(Arc::new(TestEmbeddingProvider), identity("model-b")),
        "a provider for a different vector space must not be installed"
    );
    let unchanged = runtime.snapshot().expect("retained snapshot");
    assert_eq!(Arc::as_ptr(&unchanged.index), index_ptr);
    assert_eq!(
        unchanged
            .identity
            .as_ref()
            .map(|identity| identity.model.as_str()),
        Some("model-a")
    );

    assert!(runtime.replace_provider(Arc::new(TestEmbeddingProvider), identity("model-a")));
    let after = runtime.snapshot().expect("replaced snapshot");
    assert_eq!(Arc::as_ptr(&after.index), index_ptr);
    assert_eq!(after.generation, before.generation);
}

#[test]
fn failures_are_classified_without_leaking_credentials() {
    let transient = anyhow::anyhow!("qdrant collection_exists failed: transport error");
    assert_eq!(classify_failure(&transient), VectorFailureClass::Transient);

    for permanent in [
        anyhow::anyhow!("qdrant collection c dimension mismatch: expected 8, found 16"),
        anyhow::anyhow!("request failed: Unauthenticated invalid api key sk-secret"),
        anyhow::anyhow!("configuration: embedding runtime is not configured"),
    ] {
        assert_eq!(
            classify_failure(&permanent),
            VectorFailureClass::Configuration,
            "{permanent}"
        );
    }
    // The classifier never returns the message itself.
    let message = format!(
        "{:?}",
        classify_failure(&anyhow::anyhow!("Unauthenticated sk-secret"))
    );
    assert!(!message.contains("sk-secret"));
}

#[test]
fn commit_readiness_only_admits_a_published_unfenced_pair() {
    let flag = Arc::new(AtomicBool::new(false));
    let runtime = VectorRuntime::configured(flag.clone());
    runtime.commit_readiness(true);
    assert!(!runtime.is_ready(), "no pair is published yet");
    assert!(!flag.load(Ordering::Acquire));

    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    assert!(runtime.is_ready());
    assert!(flag.load(Ordering::Acquire));

    runtime.begin_rebuild();
    assert!(!runtime.is_ready());
    runtime.commit_readiness(true);
    assert!(!runtime.is_ready(), "a rebuild fence stays closed");
    assert!(!flag.load(Ordering::Acquire));
}

#[test]
fn a_stale_rebuild_completion_cannot_reopen_a_newer_generation() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let rebuild = runtime.begin_rebuild();
    assert!(runtime.publish(rebuild, snapshot(rebuild, "model-a")));
    assert!(!runtime.is_ready(), "a published pair stays fenced");

    let newer = runtime.begin_generation();
    assert!(!runtime.complete_rebuild(rebuild));
    assert!(!runtime.is_ready());
    assert!(!runtime.is_rebuilding());
    assert_eq!(runtime.generation(), newer);
    assert!(!runtime.restore_ready(rebuild));
}

#[test]
fn a_stale_readiness_restore_is_rejected() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let first = runtime.generation();
    assert!(runtime.publish(first, snapshot(first, "model-a")));

    let newer = runtime.begin_generation();
    assert!(!runtime.restore_ready(first));
    assert!(!runtime.is_ready());
    assert_eq!(runtime.generation(), newer);
}

#[test]
fn a_ready_read_returns_the_pair_of_its_own_generation() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));

    let ready = runtime.require_ready().expect("published pair is ready");
    assert_eq!(ready.generation, generation);
    assert_eq!(
        ready
            .identity
            .as_ref()
            .map(|identity| identity.model.as_str()),
        Some("model-a")
    );

    let rebuild = runtime.begin_rebuild();
    let refused = match runtime.require_ready() {
        Ok(_) => panic!("a rebuild fence must refuse new work"),
        Err(error) => error,
    };
    assert!(refused.to_string().contains("rebuilding"), "{refused}");

    assert!(runtime.complete_rebuild(rebuild));
    assert_eq!(
        runtime
            .require_ready()
            .expect("completion reopens the same pair")
            .generation,
        rebuild
    );
}

#[test]
fn concurrent_generations_never_expose_an_incoherent_ready_pair() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let rounds = 200;

    std::thread::scope(|scope| {
        for _ in 0..4 {
            let runtime = runtime.clone();
            scope.spawn(move || {
                for _ in 0..rounds {
                    let generation = runtime.begin_generation();
                    // A concurrent generation may supersede this one first.
                    let _ = runtime.publish(generation, snapshot(generation, "model-a"));
                }
            });
        }
        for _ in 0..2 {
            let runtime = runtime.clone();
            scope.spawn(move || {
                for _ in 0..rounds {
                    let state = runtime.current();
                    if state.ready {
                        assert!(!state.rebuilding, "ready and rebuilding are exclusive");
                        let snapshot = state.snapshot.expect("ready implies a published pair");
                        assert_eq!(
                            snapshot.generation, state.generation,
                            "a ready read exposes only its own generation"
                        );
                    }
                    if let Ok(snapshot) = runtime.require_ready() {
                        assert_eq!(snapshot.collection, "test");
                    }
                }
            });
        }
    });

    let state = runtime.current();
    assert!(state.ready);
    assert_eq!(
        state.snapshot.expect("published pair").generation,
        state.generation
    );
}
