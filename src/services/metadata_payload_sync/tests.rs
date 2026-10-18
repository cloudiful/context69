use std::sync::{Arc, atomic::AtomicBool};

use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;

use crate::{
    config::EmbeddingConfig,
    embedding::build_provider,
    qdrant_index::QdrantIndex,
    vector_runtime::{VectorRuntime, VectorSnapshot},
};

use super::{FALLBACK_INTERVAL, next_tick, ready_pair};

fn provider() -> Arc<dyn crate::embedding::EmbeddingProvider> {
    build_provider(&EmbeddingConfig {
        base_url: "https://embeddings.example/v1".to_string(),
        api_key: None,
        model: "model-a".to_string(),
        dimensions: 8,
        timeout: Duration::from_secs(5),
    })
    .expect("build a provider without network access")
}

fn publish(runtime: &VectorRuntime, generation: u64) {
    assert!(runtime.publish(
        generation,
        VectorSnapshot {
            embedding: provider(),
            identity: None,
            index: Arc::new(QdrantIndex::for_test("context69", 8)),
            collection: "context69".to_string(),
            generation,
        },
    ));
}

fn runtime_with_pair() -> VectorRuntime {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let generation = runtime.generation();
    publish(&runtime, generation);
    runtime
}

#[test]
fn a_missing_pair_waits_and_a_late_publication_is_picked_up() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    assert!(ready_pair(&runtime).is_none(), "no pair yet");

    publish(&runtime, runtime.generation());
    assert!(ready_pair(&runtime).is_some(), "published pair is usable");
}

#[test]
fn a_rebuild_fences_the_worker_until_its_generation_completes() {
    let runtime = runtime_with_pair();
    assert_eq!(
        ready_pair(&runtime).expect("ready").generation,
        runtime.generation()
    );

    let fence = runtime.begin_rebuild();
    assert!(ready_pair(&runtime).is_none(), "a rebuild fence waits");
    assert!(runtime.complete_rebuild(fence));
    assert_eq!(ready_pair(&runtime).expect("unfenced").generation, fence);
}

#[test]
fn a_new_generation_waits_for_its_own_pair() {
    let runtime = runtime_with_pair();
    let newer = runtime.begin_generation();
    assert!(
        ready_pair(&runtime).is_none(),
        "a new generation is not ready"
    );

    publish(&runtime, newer);
    assert_eq!(ready_pair(&runtime).expect("new pair").generation, newer);
}

#[tokio::test]
async fn an_unready_runtime_keeps_waiting_instead_of_stopping_the_worker() {
    let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(false)));
    let shutdown = CancellationToken::new();
    let mut ticker = interval(FALLBACK_INTERVAL);

    // One tick with no pair must not end the loop; the worker just waits again.
    assert!(ready_pair(&runtime).is_none());
    assert!(next_tick(&mut ticker, &shutdown).await);
    assert!(ready_pair(&runtime).is_none());
    assert!(next_tick(&mut ticker, &shutdown).await);
}

#[tokio::test]
async fn cancelling_the_owned_token_ends_the_worker_without_waiting_for_a_tick() {
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let mut ticker = interval(Duration::from_secs(3600));

    let started = std::time::Instant::now();
    assert!(!next_tick(&mut ticker, &shutdown).await);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "cancellation must not wait for the fallback interval"
    );
}
