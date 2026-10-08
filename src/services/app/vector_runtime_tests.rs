//! Tests for the live vector runtime: the settings guard that rejects a change
//! the running runtime cannot apply, and the readiness gate a rebuild closes.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::{EmbeddingSettingsGuard, VectorIndexGate};
use crate::contracts::UpdateRuntimeEmbeddingSettings;
use crate::embedding::{EmbeddingIdentity, EmbeddingRuntime};

fn update(
    base_url: &str,
    model: &str,
    dimensions: usize,
    api_key: Option<&str>,
) -> UpdateRuntimeEmbeddingSettings {
    UpdateRuntimeEmbeddingSettings {
        base_url: base_url.to_string(),
        model: model.to_string(),
        dimensions,
        timeout_secs: 30,
        api_key: api_key.map(str::to_string),
    }
}

fn guard_with_identity(index_present: bool) -> (EmbeddingSettingsGuard, Arc<AtomicBool>) {
    let runtime = EmbeddingRuntime::new(
        None,
        Some(EmbeddingIdentity {
            base_url: "https://embeddings.example/v1".to_string(),
            model: "model-a".to_string(),
            dimensions: 8,
        }),
    );
    let reloading = Arc::new(AtomicBool::new(false));
    (
        EmbeddingSettingsGuard::new(runtime, index_present, reloading.clone()),
        reloading,
    )
}

#[test]
fn an_identity_change_is_rejected_while_the_index_is_live() {
    let (guard, _) = guard_with_identity(true);

    for candidate in [
        update("https://other.example/v1", "model-a", 8, None),
        update("https://embeddings.example/v1", "model-b", 8, None),
        update("https://embeddings.example/v1", "model-a", 16, None),
    ] {
        let error = guard
            .check(&candidate)
            .expect_err("an identity change must be rejected");
        assert!(error.to_string().contains("cannot change"), "{error}");
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
        ))
        .expect("a credential change applies live");
    // Trailing slash and surrounding whitespace are the same endpoint.
    guard
        .check(&update(
            "  https://embeddings.example/v1/  ",
            "model-a",
            8,
            None,
        ))
        .expect("an equivalent endpoint is not a change");
}

#[test]
fn an_identity_change_is_allowed_without_a_live_index() {
    let (guard, _) = guard_with_identity(false);
    guard
        .check(&update("https://other.example/v1", "model-b", 16, None))
        .expect("without an index there is nothing to keep homogeneous");
}

#[test]
fn an_identity_change_is_rejected_while_a_reload_or_rebuild_is_in_progress() {
    // Without a live index an identity change is otherwise acceptable, so
    // the reload/rebuild check is what makes this rejection observable.
    let runtime = EmbeddingRuntime::new(
        None,
        Some(EmbeddingIdentity {
            base_url: "https://embeddings.example/v1".to_string(),
            model: "model-a".to_string(),
            dimensions: 8,
        }),
    );
    let reloading = Arc::new(AtomicBool::new(true));
    let guard = EmbeddingSettingsGuard::new(runtime.clone(), false, reloading);
    let error = guard
        .check(&update("https://embeddings.example/v1", "model-b", 8, None))
        .expect_err("an identity change racing a reload must be rejected");
    assert!(error.to_string().contains("in progress"), "{error}");

    // A credential-only save stays live-safe even while a reload is running,
    // so a broken credential can still be repaired.
    let reloading = Arc::new(AtomicBool::new(true));
    let guard = EmbeddingSettingsGuard::new(runtime, false, reloading);
    guard
        .check(&update(
            "https://embeddings.example/v1",
            "model-a",
            8,
            Some("rotated-key"),
        ))
        .expect("a credential change is live-safe");
}

#[test]
fn rebuild_ticket_marks_unready_and_a_startup_gate_reopens_it() {
    let gate = VectorIndexGate::new();
    assert!(gate.flag().load(Ordering::Acquire));

    let ticket = gate.rebuild_ticket();
    assert!(!gate.flag().load(Ordering::Acquire));

    ticket.settle();
    assert!(gate.flag().load(Ordering::Acquire));
}
