//! Tests for the live vector runtime: the settings guard that rejects a change
//! the running runtime cannot apply, the shared transition exclusion, and the
//! readiness gate a rebuild closes.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::{EmbeddingSettingsGuard, VectorIndexGate};
use crate::contracts::UpdateRuntimeEmbeddingSettings;
use crate::embedding::{EmbeddingIdentity, EmbeddingRuntime};
use crate::qdrant_index::QdrantIndex;
use crate::vector_runtime::VectorRuntime;
use crate::vector_runtime::{UnrecordedCollection, decide_unrecorded_collection};

fn update(
    base_url: &str,
    model: &str,
    dimensions: usize,
    api_key: Option<&str>,
    confirm: bool,
) -> UpdateRuntimeEmbeddingSettings {
    UpdateRuntimeEmbeddingSettings {
        base_url: base_url.to_string(),
        model: model.to_string(),
        dimensions,
        timeout_secs: 30,
        api_key: api_key.map(str::to_string),
        confirm_vector_space_unchanged: confirm,
    }
}

fn identity() -> EmbeddingIdentity {
    EmbeddingIdentity {
        base_url: "https://embeddings.example/v1".to_string(),
        model: "model-a".to_string(),
        dimensions: 8,
    }
}

fn guard_with_identity(index_present: bool) -> (EmbeddingSettingsGuard, Arc<AtomicBool>) {
    let runtime = EmbeddingRuntime::new(None, Some(identity()));
    let vector = if index_present {
        VectorRuntime::for_test_with_index(QdrantIndex::for_test("test", 8), identity())
    } else {
        VectorRuntime::unconfigured()
    };
    let reloading = Arc::new(AtomicBool::new(false));
    (
        EmbeddingSettingsGuard::new(runtime, vector, reloading.clone()),
        reloading,
    )
}

#[test]
fn an_unconfirmed_identity_change_is_rejected_while_the_index_is_live() {
    let (guard, _) = guard_with_identity(true);

    for candidate in [
        update("https://other.example/v1", "model-a", 8, None, false),
        update("https://embeddings.example/v1", "model-b", 8, None, false),
    ] {
        let error = guard
            .check(&candidate)
            .expect_err("an unconfirmed identity change must be rejected");
        assert!(error.to_string().contains("cannot change"), "{error}");
        assert!(
            error.to_string().contains("confirmation"),
            "the error must ask for an explicit confirmation: {error}"
        );
    }

    // A credential or timeout change keeps the identity and is allowed. The
    // timeout is not part of the update struct's identity comparison, so a
    // key-only change is the representative live-safe case.
    guard
        .check(&update(
            "https://embeddings.example/v1",
            "model-a",
            8,
            Some("rotated-key"),
            false,
        ))
        .expect("a credential change applies live");
    // Trailing slash and surrounding whitespace are the same endpoint.
    guard
        .check(&update(
            "  https://embeddings.example/v1/  ",
            "model-a",
            8,
            None,
            false,
        ))
        .expect("an equivalent endpoint is not a change");
}

#[test]
fn a_confirmed_same_dimension_change_is_allowed_on_a_ready_index() {
    let (guard, _) = guard_with_identity(true);

    guard
        .check(&update(
            "https://gateway.example/v1",
            "alias-model",
            8,
            Some("rotated-key"),
            true,
        ))
        .expect("a confirmed same-dimensional alias plus credential change is allowed");
}

#[test]
fn a_dimension_change_is_rejected_even_with_confirmation() {
    let (guard, _) = guard_with_identity(true);

    let error = guard
        .check(&update(
            "https://embeddings.example/v1",
            "model-a",
            16,
            None,
            true,
        ))
        .expect_err("a dimension change must never be confirmable");
    assert!(error.to_string().contains("dimensions"), "{error}");
    assert!(error.to_string().contains("cannot change"), "{error}");
}

#[test]
fn confirmed_change_is_rejected_while_a_reload_or_rebuild_is_in_progress() {
    let runtime = EmbeddingRuntime::new(None, Some(identity()));
    let vector = VectorRuntime::for_test_with_index(QdrantIndex::for_test("test", 8), identity());
    let reloading = Arc::new(AtomicBool::new(true));
    let guard = EmbeddingSettingsGuard::new(runtime.clone(), vector.clone(), reloading);
    let error = guard
        .check(&update(
            "https://gateway.example/v1",
            "alias-model",
            8,
            None,
            true,
        ))
        .expect_err("a confirmed change racing a reload must be rejected");
    assert!(error.to_string().contains("in progress"), "{error}");

    // A credential-only save stays live-safe even while a reload is running,
    // so a broken credential can still be repaired.
    let reloading = Arc::new(AtomicBool::new(true));
    let guard = EmbeddingSettingsGuard::new(runtime, vector, reloading);
    guard
        .check(&update(
            "https://embeddings.example/v1",
            "model-a",
            8,
            Some("rotated-key"),
            false,
        ))
        .expect("a credential change is live-safe");
}

#[test]
fn confirmed_change_is_rejected_while_another_transition_owns_the_exclusion() {
    let runtime = EmbeddingRuntime::new(None, Some(identity()));
    let vector = VectorRuntime::for_test_with_index(QdrantIndex::for_test("test", 8), identity());
    let guard =
        EmbeddingSettingsGuard::new(runtime, vector.clone(), Arc::new(AtomicBool::new(false)));
    assert!(vector.try_begin_identity_transition());
    let error = guard
        .check(&update(
            "https://gateway.example/v1",
            "alias-model",
            8,
            None,
            true,
        ))
        .expect_err("a concurrent transition must reject the save");
    assert!(error.to_string().contains("in progress"), "{error}");
    vector.end_identity_transition();
}

#[test]
fn an_identity_change_is_allowed_for_first_configuration() {
    let runtime = EmbeddingRuntime::new(None, None);
    let vector = VectorRuntime::unconfigured();
    let guard = EmbeddingSettingsGuard::new(runtime, vector, Arc::new(AtomicBool::new(false)));
    guard
        .check(&update(
            "https://other.example/v1",
            "model-b",
            16,
            None,
            false,
        ))
        .expect("without a live identity there is nothing to mix");
}

#[test]
fn a_rebuild_fence_closes_and_reopens_through_committed_state() {
    let runtime = VectorRuntime::for_test_with_index(QdrantIndex::for_test("test", 8), identity());
    assert!(runtime.is_ready());
    let gate = VectorIndexGate::from_runtime(&runtime);
    assert!(gate.flag().load(Ordering::Acquire));

    let generation = runtime.begin_rebuild();
    assert!(!gate.flag().load(Ordering::Acquire));
    assert!(!runtime.is_ready());
    assert!(runtime.is_rebuilding());

    assert!(runtime.complete_rebuild(generation));
    assert!(gate.flag().load(Ordering::Acquire));
    assert!(runtime.is_ready());
}

#[test]
fn the_startup_truth_table_only_requires_a_rebuild_for_a_persisted_mismatch() {
    assert!(
        !super::requires_rebuild_from_persisted(false, Some("stale"), "configured"),
        "an unconfigured runtime has no vector work to gate"
    );
    assert!(
        !super::requires_rebuild_from_persisted(true, None, "configured"),
        "a missing persisted identity is the safe zero-data bootstrap path"
    );
    assert!(
        !super::requires_rebuild_from_persisted(true, Some("configured"), "configured"),
        "a matching legacy identity stays ready with no upgrade-wide rebuild"
    );
    assert!(
        super::requires_rebuild_from_persisted(true, Some("stale"), "configured"),
        "a persisted mismatch requires an explicit rebuild"
    );
}

#[test]
fn an_unrecorded_collection_is_only_created_or_adopted_when_it_is_genuinely_empty() {
    assert_eq!(
        decide_unrecorded_collection(None, 8),
        UnrecordedCollection::CreateMissing,
        "an absent collection is the safe zero-data bootstrap path"
    );
    assert_eq!(
        decide_unrecorded_collection(Some((0, 8)), 8),
        UnrecordedCollection::AdoptEmpty,
        "an empty collection at the configured dimension may be adopted"
    );
    assert_eq!(
        decide_unrecorded_collection(Some((7, 8)), 8),
        UnrecordedCollection::RebuildOnly { points: 7 },
        "populated vectors without a recorded identity require an explicit rebuild"
    );
    assert_eq!(
        decide_unrecorded_collection(Some((0, 16)), 8),
        UnrecordedCollection::BlockedDimensions {
            found: 16,
            points: 0
        },
        "an empty collection of another dimension is not a compatible space"
    );
    assert_eq!(
        decide_unrecorded_collection(Some((42, 16)), 8),
        UnrecordedCollection::BlockedDimensions {
            found: 16,
            points: 42
        },
        "a populated collection of another dimension is not a compatible space"
    );
}

#[test]
fn the_startup_path_never_consults_the_legacy_recreate_toggle() {
    // The decision has no toggle input: the legacy configuration field is kept
    // only for wire compatibility, so no value of it can turn a populated
    // collection into deletion, recreation, or adoption. A guard rather than a
    // comment, because re-reading the flag here would silently restore the old
    // behaviour.
    let startup_path = include_str!("vector_recovery.rs");
    assert!(
        !startup_path.contains(".recreate_on_dimension_mismatch"),
        "startup recovery must not consult the legacy recreate toggle"
    );
}

#[test]
fn the_shared_fingerprint_helper_matches_the_persisted_format() {
    let config = crate::config::Config::default();
    assert_eq!(
        crate::embedding::embedding_runtime::vector_identity_fingerprint(
            &config.qdrant.url,
            &config.qdrant.collection_name,
            &config.embedding.base_url,
            &config.embedding.model,
            config.embedding.dimensions,
        ),
        super::super::vector_identity::fingerprint(&config),
        "the settings acknowledgement must use the persisted fingerprint format"
    );
}
