//! Tests for the embedding runtime's readiness gate and cache identity: a
//! rebuild closes the shared flag for every consumer, and the cache identity
//! moves only when the vector space does.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::EmbeddingRuntime;
use super::test_fixtures::{CountingProvider, identity};
use crate::embedding::EmbeddingProvider;

#[tokio::test]
async fn require_ready_refuses_while_the_index_is_rebuilding() {
    let ready = Arc::new(AtomicBool::new(true));
    let runtime = EmbeddingRuntime::with_readiness(
        Some(Arc::new(CountingProvider::new(2))),
        Some(identity("model-a", 2)),
        ready.clone(),
    );

    assert!(runtime.require_ready().is_ok());

    ready.store(false, Ordering::Release);
    let error = match runtime.require_ready() {
        Err(error) => error,
        Ok(_) => panic!("an unready handle must refuse a writer"),
    };
    assert!(error.to_string().contains("rebuilding"), "{error}");
    // The rebuild itself reads through `require`, which is not gated.
    assert!(runtime.require().is_ok());

    ready.store(true, Ordering::Release);
    assert!(runtime.require_ready().is_ok());
}

#[test]
fn cache_identity_tracks_the_installed_identity() {
    let runtime = EmbeddingRuntime::default();
    assert_eq!(runtime.cache_identity(), "unconfigured");

    runtime.install(Arc::new(CountingProvider::new(2)), identity("model-a", 2));
    let first = runtime.cache_identity();
    assert!(first.contains("model-a"), "{first}");

    runtime.install(Arc::new(CountingProvider::new(2)), identity("model-b", 2));
    assert_ne!(first, runtime.cache_identity());
}

/// Writer gating only works if every consumer holds the *same* readiness flag. A
/// clone that carried its own copy would silently keep writing into a collection
/// that is being re-embedded, so the shared cell is asserted directly.
#[tokio::test]
async fn a_clone_observes_the_shared_rebuild_gate() {
    let ready = Arc::new(AtomicBool::new(true));
    let runtime = EmbeddingRuntime::with_readiness(
        Some(Arc::new(CountingProvider::new(2))),
        Some(identity("model-a", 2)),
        ready.clone(),
    );
    // A consumer holds a clone, the way every service stores its handle.
    let consumer = runtime.clone();

    assert!(Arc::ptr_eq(
        &consumer.readiness_flag(),
        &runtime.readiness_flag()
    ));

    // An identity-changing rebuild closes the gate from outside the handle.
    ready.store(false, Ordering::Release);
    assert!(!consumer.is_ready());
    assert!(consumer.require_ready().is_err());

    // The provider and its identity stay readable while unready: the reloader and
    // the live query-cache identity still need them to decide, and an in-flight
    // request that already acquired the provider must not be disturbed.
    assert!(consumer.current().is_some());
    assert_eq!(consumer.current_identity(), Some(identity("model-a", 2)));
    assert!(consumer.cache_identity().contains("model-a"));
    let acquired = consumer.require().expect("require is not gated");
    assert_eq!(
        acquired
            .embed_texts(&["in-flight".to_string()])
            .await
            .unwrap()[0]
            .len(),
        2
    );

    ready.store(true, Ordering::Release);
    assert!(consumer.require_ready().is_ok());
}

/// A handle with no identity must still produce a distinct cache identity per
/// provider, so two identity-less providers can never share cached vectors.
#[test]
fn an_identityless_handle_scopes_its_cache_identity_per_provider() {
    let first: EmbeddingRuntime =
        Some(Arc::new(CountingProvider::new(2)) as Arc<dyn EmbeddingProvider>).into();
    let second: EmbeddingRuntime =
        Some(Arc::new(CountingProvider::new(2)) as Arc<dyn EmbeddingProvider>).into();

    assert!(
        first.cache_identity().starts_with("provider:"),
        "{:?}",
        first.cache_identity()
    );
    assert_ne!(
        first.cache_identity(),
        second.cache_identity(),
        "two distinct providers must not share a query-embedding cache entry"
    );

    // A clone is the same provider, so it keeps the same cache identity.
    assert_eq!(first.cache_identity(), first.clone().cache_identity());

    // A credential-only change keeps the same identity, so the cache survives it.
    let live = EmbeddingRuntime::with_readiness(
        Some(Arc::new(CountingProvider::new(2))),
        Some(identity("model-a", 2)),
        Arc::new(AtomicBool::new(true)),
    );
    let before = live.cache_identity();
    live.install(Arc::new(CountingProvider::new(2)), identity("model-a", 2));
    assert_eq!(
        before,
        live.cache_identity(),
        "a rebuild-worthy identity change is what must move the cache"
    );
}
