//! Tests for the embedding runtime handle's replacement contract: an installed
//! provider and its identity become visible together, a request in flight keeps
//! the provider it started with, and a credential/timeout swap leaves the
//! vector space untouched.

use std::sync::Arc;

use tokio::sync::Notify;

use super::test_fixtures::{BlockingProvider, CountingProvider, identity};
use super::{EmbeddingIdentity, EmbeddingRuntime, build_probe_provider};
use crate::embedding::EmbeddingProvider;

/// The concurrency contract is that the read lock is released before any network
/// I/O, not merely that an acquired `Arc` survives: a save must be able to
/// install a replacement while a request is still blocked inside its provider.
#[tokio::test]
async fn a_replacement_installs_while_a_request_is_in_flight() {
    let runtime = EmbeddingRuntime::default();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    runtime.install(
        Arc::new(BlockingProvider {
            width: 2,
            started: started.clone(),
            release: release.clone(),
        }),
        identity("model-a", 2),
    );

    let in_flight = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .current()
                .expect("configured")
                .embed_texts(&["in-flight".to_string()])
                .await
        }
    });
    started.notified().await;

    // The in-flight request is parked inside its provider right now. An install
    // that waited on it would deadlock here instead of completing.
    let replacement = Arc::new(CountingProvider::new(2));
    let installed = tokio::time::timeout(std::time::Duration::from_secs(5), {
        let runtime = runtime.clone();
        let replacement = replacement.clone();
        async move { runtime.install(replacement, identity("model-b", 2)) }
    })
    .await;
    assert!(
        installed.is_ok(),
        "install must not wait for in-flight embedding network I/O"
    );

    // A request started after the swap acquires the replacement and completes on
    // it without waiting for the parked one.
    let after = runtime
        .current()
        .expect("configured")
        .embed_texts(&["next".to_string()])
        .await
        .expect("the replacement serves new requests");
    assert_eq!(after[0].len(), 2);
    assert_eq!(runtime.current_identity(), Some(identity("model-b", 2)));
    assert_eq!(replacement.calls(), 1);

    // The parked request still completes on the provider it acquired.
    release.notify_one();
    let finished = in_flight
        .await
        .expect("the in-flight task joins")
        .expect("the released request succeeds");
    assert_eq!(finished[0].len(), 2);
    assert_eq!(
        replacement.calls(),
        1,
        "the parked request never borrowed the replacement"
    );
}

/// The mirror case: an install that has not happened yet must not be visible to a
/// clone that is mid-request, and the identity must never be reported for a
/// provider the handle never installed.
#[tokio::test]
async fn a_rejected_probe_candidate_never_reaches_an_installed_handle() {
    let runtime = EmbeddingRuntime::default();
    let first = Arc::new(CountingProvider::new(2));
    runtime.install(first.clone(), identity("model-a", 2));

    // A candidate that fails validation is an `Err`; the handle is only ever
    // written through `install`, so the previous provider and its identity stay.
    let rejected = build_probe_provider("  ", "model-b", 2, 5, None);
    assert!(
        rejected.is_err(),
        "an empty base URL must be rejected before a provider is built"
    );
    drop(rejected);

    assert_eq!(runtime.current_identity(), Some(identity("model-a", 2)));
    let current = runtime
        .current()
        .expect("the previous runtime stays usable");
    let vectors = current
        .embed_texts(&["still-old".to_string()])
        .await
        .unwrap();
    assert_eq!(vectors[0].len(), 2);
    assert_eq!(first.calls(), 1);
}

#[tokio::test]
async fn a_replacement_is_visible_through_every_clone() {
    let runtime = EmbeddingRuntime::default();
    let clone = runtime.clone();
    assert!(!clone.is_configured());

    runtime.install(Arc::new(CountingProvider::new(3)), identity("model-a", 3));

    assert!(clone.is_configured());
    let current = clone.current().expect("clone sees the installed provider");
    let vectors = current.embed_texts(&["one".to_string()]).await.unwrap();
    assert_eq!(vectors[0].len(), 3);
    assert_eq!(clone.current_identity(), Some(identity("model-a", 3)));
}

#[tokio::test]
async fn an_in_flight_request_keeps_the_provider_it_started_with() {
    let runtime = EmbeddingRuntime::default();
    let first = Arc::new(CountingProvider::new(2));
    runtime.install(first.clone(), identity("model-a", 2));

    // A request acquires the provider before settings are saved.
    let acquired = runtime.current().expect("configured");

    // The save installs a replacement.
    let second = Arc::new(CountingProvider::new(2));
    runtime.install(second.clone(), identity("model-b", 2));

    // The in-flight request still completes on the provider it acquired ...
    let vectors = acquired
        .embed_texts(&["in-flight".to_string()])
        .await
        .unwrap();
    assert_eq!(vectors.len(), 1);
    assert_eq!(
        first.calls(),
        1,
        "the in-flight provider served the request"
    );
    assert_eq!(
        second.calls(),
        0,
        "the replacement is not borrowed mid-request"
    );

    // ... while the next request acquires the replacement.
    runtime
        .current()
        .expect("configured")
        .embed_texts(&["next".to_string()])
        .await
        .unwrap();
    assert_eq!(second.calls(), 1);
    assert_eq!(first.calls(), 1);
}

#[tokio::test]
async fn a_bare_provider_option_adapts_to_an_identityless_handle() {
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(CountingProvider::new(4));
    let runtime: EmbeddingRuntime = Some(provider).into();

    assert!(runtime.is_configured());
    assert_eq!(runtime.current_identity(), None);

    let none: EmbeddingRuntime = None.into();
    assert!(!none.is_configured());
    assert!(none.current().is_none());
    assert!(none.require().is_err());
}

#[test]
fn identity_from_update_normalizes_and_excludes_credentials() {
    use crate::contracts::UpdateRuntimeEmbeddingSettings;

    let update = UpdateRuntimeEmbeddingSettings {
        base_url: "  https://embeddings.example/v1/  ".to_string(),
        model: " model-a ".to_string(),
        dimensions: 8,
        timeout_secs: 5,
        api_key: Some("a-rotated-key".to_string()),
    };
    assert_eq!(
        EmbeddingIdentity::from_update(&update),
        identity("model-a", 8)
    );

    // A credential-only update carries the same identity, so it is live-safe.
    let rotated = UpdateRuntimeEmbeddingSettings {
        api_key: Some("another-key".to_string()),
        ..update.clone()
    };
    assert_eq!(
        EmbeddingIdentity::from_update(&rotated),
        EmbeddingIdentity::from_update(&update),
        "the API key is not part of the identity"
    );

    // The timeout is the other hot-swappable field, and it is named as such by
    // the acceptance criteria, so a timeout-only save must not read as an
    // identity change either.
    for timeout_secs in [1, 30, 300] {
        let retimed = UpdateRuntimeEmbeddingSettings {
            timeout_secs,
            ..update.clone()
        };
        assert_eq!(
            EmbeddingIdentity::from_update(&retimed),
            EmbeddingIdentity::from_update(&update),
            "a timeout of {timeout_secs}s is not part of the identity"
        );
    }
}

/// The narrowed contract: a credential or timeout save swaps the provider
/// underneath every consumer while leaving the vector space - and therefore the
/// collection and the query cache - exactly as it was. This is the invariant the
/// whole hot-swap rests on, so it is asserted directly rather than inferred
/// from the guard.
#[tokio::test]
async fn a_credential_or_timeout_swap_replaces_the_provider_and_keeps_the_identity() {
    let runtime = EmbeddingRuntime::default();
    let live_identity = identity("model-a", 4);

    let before = Arc::new(CountingProvider::new(4));
    runtime.install(before.clone(), live_identity.clone());
    let identity_before = runtime.current_identity();
    let cache_before = runtime.cache_identity();

    // The rotation installs a genuinely different provider object.
    let after = Arc::new(CountingProvider::new(4));
    runtime.install(after.clone(), live_identity.clone());

    // A consumer clone sees the new provider ...
    let consumer = runtime.clone();
    assert_ne!(
        Arc::as_ptr(&consumer.current().expect("configured")) as *const (),
        Arc::as_ptr(&before) as *const (),
        "the rotated provider must be the one consumers acquire"
    );
    // ... while the vector space is untouched.
    assert_eq!(consumer.current_identity(), identity_before);
    assert_eq!(
        consumer.cache_identity(),
        cache_before,
        "a credential or timeout save must not invalidate cached query vectors"
    );

    // Both providers agree on the width, so the collection stays homogeneous.
    let vectors = consumer
        .current()
        .expect("configured")
        .embed_texts(&["after-rotation".to_string()])
        .await
        .expect("the rotated provider serves requests");
    assert_eq!(vectors[0].len(), 4);
    assert_eq!(after.calls(), 1);
    assert_eq!(before.calls(), 0);
}
