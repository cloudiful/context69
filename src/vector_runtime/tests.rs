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
fn a_same_space_rekey_preserves_the_index_generation_and_collection() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    let before = runtime.snapshot().expect("published pair");
    let index_ptr = Arc::as_ptr(&before.index);
    let cache_before = runtime.cache_identity();

    let replacement = Arc::new(TestEmbeddingProvider);
    assert!(runtime.rekey_same_space(replacement, identity("model-alias")));

    let after = runtime.snapshot().expect("rekeyed pair");
    assert_eq!(after.generation, generation, "the generation is preserved");
    assert_eq!(
        Arc::as_ptr(&after.index),
        index_ptr,
        "the index is preserved"
    );
    assert_eq!(after.collection, "test");
    assert_eq!(
        after
            .identity
            .as_ref()
            .map(|identity| identity.model.as_str()),
        Some("model-alias")
    );
    assert_ne!(
        runtime.cache_identity(),
        cache_before,
        "cached query vectors are scoped to the new identity"
    );
    assert!(runtime.is_ready(), "the pair stays ready through the rekey");
}

#[test]
fn a_same_space_rekey_rejects_a_dimension_change_rebuild_or_missing_pair() {
    let unconfigured = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    assert!(
        !unconfigured.rekey_same_space(Arc::new(TestEmbeddingProvider), identity("model-a")),
        "no published pair cannot be rekeyed"
    );

    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    assert!(
        !runtime.rekey_same_space(
            Arc::new(TestEmbeddingProvider),
            EmbeddingIdentity {
                dimensions: 16,
                ..identity("model-a")
            }
        ),
        "a different dimension must never rekey the live pair"
    );
    let rebuild = runtime.begin_rebuild();
    assert!(
        !runtime.rekey_same_space(Arc::new(TestEmbeddingProvider), identity("model-a")),
        "a rebuild fence rejects the rekey"
    );
    assert!(runtime.fail_rebuild(rebuild));
}

#[test]
fn an_identity_transition_and_a_manual_rebuild_start_are_mutually_exclusive() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));

    assert!(runtime.try_begin_identity_transition());
    assert!(
        runtime.try_begin_rebuild().is_none(),
        "a manual rebuild cannot start during an identity transition"
    );
    runtime.end_identity_transition();

    let rebuild = runtime.try_begin_rebuild().expect("the fence opens");
    assert!(runtime.is_rebuilding());
    assert!(
        !runtime.try_begin_identity_transition(),
        "an identity transition cannot start during a rebuild"
    );
    assert!(
        runtime.try_begin_rebuild().is_none(),
        "a duplicate manual rebuild start is rejected"
    );
    assert!(runtime.complete_rebuild(rebuild));
    assert!(runtime.try_begin_identity_transition());
    runtime.end_identity_transition();
}

#[test]
fn a_persisted_identity_mismatch_keeps_readiness_off_until_an_explicit_success() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.set_requires_rebuild(true);

    // Publishing a probeable pair is not readiness for a mismatched space.
    assert!(runtime.publish(generation, snapshot(generation, "model-a")));
    assert!(!runtime.is_ready());
    assert!(runtime.requires_rebuild());
    assert!(
        runtime.require_ready().is_err(),
        "a mismatched space must not serve new vector work"
    );
    assert!(
        !runtime.restore_ready(generation),
        "a probe must not reopen readiness while a rebuild is required"
    );
    runtime.commit_readiness(true);
    assert!(!runtime.is_ready());

    // Only the explicit rebuild success clears the requirement, and the fence
    // completion then reopens exactly the rebuilt pair.
    let rebuild = runtime.begin_rebuild();
    runtime.set_requires_rebuild(false);
    assert!(runtime.complete_rebuild(rebuild));
    assert!(runtime.is_ready());
    assert_eq!(
        runtime.require_ready().expect("ready pair").generation,
        rebuild
    );
}

#[test]
fn a_configured_fingerprint_is_recorded_without_affecting_readiness() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    assert_eq!(runtime.configured_fingerprint(), None);
    runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));
    assert_eq!(
        runtime.configured_fingerprint().as_deref(),
        Some("fingerprint-a")
    );
    assert!(
        !runtime.is_ready(),
        "recording a fingerprint is not readiness"
    );
}

#[test]
fn a_failed_rebuild_latches_the_requirement_from_a_ready_start() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    assert!(runtime.is_ready());
    assert!(!runtime.requires_rebuild());

    let rebuild = runtime.begin_rebuild();
    assert!(runtime.fail_rebuild(rebuild));
    assert!(!runtime.is_rebuilding());
    assert!(!runtime.is_ready());
    assert!(
        runtime.requires_rebuild(),
        "any failed explicit rebuild requires a later successful one"
    );

    // The ordinary supervisor's probe path and an application readiness commit
    // must both stay refused.
    assert!(!runtime.restore_ready(rebuild));
    runtime.commit_readiness(true);
    assert!(!runtime.is_ready());
    assert!(runtime.require_ready().is_err());

    // Only a completed explicit rebuild clears the latch.
    runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));
    let retry = runtime.try_begin_rebuild().expect("a retry is admitted");
    runtime.set_requires_rebuild(false);
    assert!(runtime.complete_rebuild(retry));
    assert!(runtime.is_ready());
    assert!(!runtime.requires_rebuild());
}

#[test]
fn a_stale_failed_rebuild_cannot_latch_a_newer_generation() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    let rebuild = runtime.begin_rebuild();

    let newer = runtime.begin_generation();
    assert!(!runtime.fail_rebuild(rebuild));
    assert!(
        !runtime.requires_rebuild(),
        "a superseded failure must not mark the newer generation"
    );
    assert_eq!(runtime.generation(), newer);
}

#[test]
fn a_rebuild_cannot_start_from_a_generation_whose_snapshot_it_did_not_read() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));

    // A destination change advances the generation and retains the old
    // snapshot for diagnostics without publishing a replacement pair. The
    // production admission must reject it instead of relabelling it.
    assert!(runtime.try_begin_identity_transition());
    let (changed, newer) = runtime
        .apply_runtime_reconfiguration(Some("fingerprint-b".to_string()), || true)
        .expect("no fence is open");
    runtime.end_identity_transition();
    assert!(changed);
    assert!(newer > generation);
    assert_eq!(
        runtime
            .try_admit_rebuild()
            .err()
            .expect("stale pair is rejected"),
        crate::vector_runtime::RebuildAdmissionError::StaleSnapshot
    );
    assert!(!runtime.is_rebuilding());

    // Once the newer destination publishes its own pair it is admitted.
    let current = runtime.generation();
    runtime.publish(current, snapshot(current, "model-b"));
    let admitted = runtime
        .try_admit_rebuild()
        .expect("the current replacement generation is admitted");
    assert_eq!(
        admitted
            .snapshot
            .identity
            .as_ref()
            .map(|identity| identity.model.as_str()),
        Some("model-b")
    );
    assert_eq!(admitted.generation, current + 1);
    assert!(runtime.is_rebuilding());
    assert!(runtime.fail_rebuild(admitted.generation));
}

#[test]
fn a_superseded_successful_settlement_clears_nothing() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));
    let rebuild = runtime.begin_rebuild();

    // A newer generation with its own requirement must not be touched.
    let newer = runtime.begin_generation();
    runtime.set_requires_rebuild(true);
    assert!(!runtime.complete_rebuild_success(rebuild));
    assert!(runtime.requires_rebuild());
    assert!(!runtime.is_ready());
    assert_eq!(runtime.generation(), newer);
    assert!(!runtime.restore_ready(newer));

    // The current generation settles normally once its own pair is published.
    runtime.publish(newer, snapshot(newer, "model-b"));
    runtime.set_configured_fingerprint(Some("fingerprint-b".to_string()));
    let admitted = runtime.try_admit_rebuild().expect("fence opens");
    assert!(runtime.complete_rebuild_success(admitted.generation));
    assert!(runtime.is_ready());
    assert!(!runtime.requires_rebuild());
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

#[test]
fn runtime_reconfiguration_and_rebuild_admission_are_one_exclusive_step() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));
    runtime.set_configured_fingerprint(Some("fingerprint-a".to_string()));

    // A rebuild that owns the runtime refuses the admission, not the other way
    // round: there is no check-then-act window.
    let rebuild = runtime.begin_rebuild();
    assert!(!runtime.try_begin_identity_transition());
    assert!(runtime.is_rebuilding());
    assert!(runtime.fail_rebuild(rebuild));
    assert!(runtime.requires_rebuild());
    runtime.set_requires_rebuild(false);
    assert!(runtime.restore_ready(rebuild));

    // While the reservation is held, a rebuild start cannot slip in.
    assert!(runtime.try_begin_identity_transition());
    assert!(runtime.try_begin_rebuild().is_none());
    let (changed, applied_generation) = runtime
        .apply_runtime_reconfiguration(Some("fingerprint-b".to_string()), || true)
        .expect("no fence is open");
    assert!(changed);
    assert!(applied_generation > generation);
    assert!(
        !runtime.is_ready(),
        "a changed destination withdraws readiness"
    );
    runtime.end_identity_transition();

    // A rebuild of the new published pair is admitted once the admission is
    // released; the retained pre-change pair alone would not be.
    runtime.publish(applied_generation, snapshot(applied_generation, "model-b"));
    assert!(runtime.try_begin_rebuild().is_some());
}

#[test]
fn an_unchanged_runtime_reconfiguration_keeps_readiness() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    runtime.publish(generation, snapshot(generation, "model-a"));

    assert!(runtime.try_begin_identity_transition());
    let (changed, applied_generation) = runtime
        .apply_runtime_reconfiguration(Some("fingerprint-a".to_string()), || false)
        .expect("no fence is open");
    runtime.end_identity_transition();
    assert!(!changed);
    assert_eq!(applied_generation, generation);
    assert!(
        runtime.is_ready(),
        "a credential-only change keeps the pair"
    );
}
