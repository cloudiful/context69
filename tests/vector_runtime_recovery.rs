//! Delayed-Qdrant recovery: a runtime that is unavailable for the whole
//! startup budget becomes usable afterwards for every consumer, and a runtime
//! that never becomes available stays honestly unavailable.
//!
//! The case uses the loopback-only test index constructor (no network) and a
//! deterministic sleeper, so it proves the recovery sequencing without a real
//! Qdrant deployment or a slow real-time wait.

#![cfg(feature = "integration-test-helpers")]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use context69::{
    embedding::{EmbeddingIdentity, EmbeddingProvider},
    qdrant_index::QdrantIndex,
    vector_runtime::{
        VectorRuntime,
        recovery::{RecoveryPolicy, Sleeper, VectorConnector, VectorPair, VectorRecovery},
    },
};
use tokio_util::sync::CancellationToken;

struct TestProvider;

#[async_trait]
impl EmbeddingProvider for TestProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; 8]).collect())
    }
}

struct DelayedConnector {
    available: AtomicBool,
    attach_calls: AtomicUsize,
    collection: Mutex<String>,
}

#[async_trait]
impl VectorConnector for DelayedConnector {
    async fn pair(&self) -> Result<VectorPair> {
        Ok(VectorPair {
            embedding: Arc::new(TestProvider),
            identity: EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
            collection: self.collection.lock().expect("collection").clone(),
        })
    }

    async fn attach(&self, pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        self.attach_calls.fetch_add(1, Ordering::SeqCst);
        if !self.available.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!(
                "qdrant collection {} is unreachable: connection refused",
                pair.collection
            ));
        }
        Ok(Arc::new(QdrantIndex::for_test_unreachable(
            "http://127.0.0.1:1",
            &pair.collection,
            8,
        )?))
    }

    async fn probe(&self, _snapshot: &context69::vector_runtime::VectorSnapshot) -> Result<()> {
        // The paired snapshot is what a probe would use; this case never
        // reaches a probe because the sleeper is stepped only after the
        // publication assertion.
        Ok(())
    }

    fn configuration_fingerprint(&self) -> String {
        "delayed-runtime".to_string()
    }
}

#[derive(Default)]
struct CountingSleeper {
    sleeps: AtomicUsize,
}

#[async_trait]
impl Sleeper for CountingSleeper {
    async fn sleep(&self, _duration: Duration) {
        self.sleeps.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
    }
}

struct StepSleeper {
    notify: tokio::sync::Notify,
}

impl StepSleeper {
    fn new() -> Self {
        Self {
            notify: tokio::sync::Notify::new(),
        }
    }

    fn step(&self) {
        self.notify.notify_one();
    }
}

#[async_trait]
impl Sleeper for StepSleeper {
    async fn sleep(&self, _duration: Duration) {
        self.notify.notified().await;
    }
}

fn configured_runtime() -> VectorRuntime {
    VectorRuntime::configured(Arc::new(AtomicBool::new(false)))
}

#[tokio::test]
async fn a_runtime_unavailable_for_the_whole_budget_heals_every_consumer_later() {
    let runtime = configured_runtime();
    let connector = Arc::new(DelayedConnector {
        available: AtomicBool::new(false),
        attach_calls: AtomicUsize::new(0),
        collection: Mutex::new("context69".to_string()),
    });
    let sleeper = Arc::new(CountingSleeper::default());
    let recovery = VectorRecovery::new(runtime.clone(), connector.clone(), sleeper.clone());

    assert!(
        !recovery.run_startup(&CancellationToken::new()).await,
        "a runtime unavailable for the whole budget must not fake a connection"
    );
    assert!(runtime.snapshot().is_none());
    assert!(!runtime.is_ready());
    assert!(runtime.require_ready().is_err());

    // The remote appears after the startup deadline.
    connector.available.store(true, Ordering::Release);
    let step = Arc::new(StepSleeper::new());
    let supervisor = VectorRecovery::new(runtime.clone(), connector.clone(), step.clone());
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { supervisor.supervise(shutdown).await }
    });

    for _ in 0..50 {
        tokio::task::yield_now().await;
        if runtime.snapshot().is_some() {
            break;
        }
    }

    let published = runtime
        .snapshot()
        .expect("the recovered runtime must publish a pair");
    assert!(runtime.is_ready());
    assert_eq!(published.generation, runtime.generation());

    // Every existing consumer takes the same coherent pair from the handle.
    for _consumer in 0..7 {
        let snapshot = runtime.require_ready().expect("consumer sees the pair");
        assert_eq!(snapshot.generation, published.generation);
        assert_eq!(Arc::as_ptr(&snapshot.index), Arc::as_ptr(&published.index));
        assert_eq!(snapshot.collection, "context69");
    }

    step.step();
    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_runtime_that_never_appears_stays_unavailable_without_a_retry_storm() {
    let runtime = configured_runtime();
    let connector = Arc::new(DelayedConnector {
        available: AtomicBool::new(false),
        attach_calls: AtomicUsize::new(0),
        collection: Mutex::new("context69".to_string()),
    });
    let sleeper = Arc::new(CountingSleeper::default());
    let policy = RecoveryPolicy {
        startup_budget: Duration::from_secs(30),
        ..RecoveryPolicy::default()
    };
    let recovery =
        VectorRecovery::new(runtime.clone(), connector.clone(), sleeper).with_policy(policy);

    assert!(!recovery.run_startup(&CancellationToken::new()).await);
    let attempts = connector.attach_calls.load(Ordering::SeqCst);
    assert!(attempts >= 1);
    assert!(
        attempts <= 6,
        "startup must stay bounded, observed {attempts} attempts"
    );
    assert!(runtime.snapshot().is_none());
    assert!(!runtime.is_ready());
}

/// A connector whose attempts are counted, so a dormant instance can prove it
/// never touched a provider or an endpoint.
struct CountingConnector {
    pair_calls: AtomicUsize,
    attach_calls: AtomicUsize,
}

impl CountingConnector {
    fn new() -> Self {
        Self {
            pair_calls: AtomicUsize::new(0),
            attach_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl VectorConnector for CountingConnector {
    async fn pair(&self) -> Result<VectorPair> {
        self.pair_calls.fetch_add(1, Ordering::SeqCst);
        Ok(VectorPair {
            embedding: Arc::new(TestProvider),
            identity: EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
            collection: "context69".to_string(),
        })
    }

    async fn attach(&self, pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        self.attach_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(QdrantIndex::for_test_unreachable(
            "http://127.0.0.1:1",
            &pair.collection,
            8,
        )?))
    }

    async fn probe(&self, _snapshot: &context69::vector_runtime::VectorSnapshot) -> Result<()> {
        Ok(())
    }

    fn configuration_fingerprint(&self) -> String {
        "counting-runtime".to_string()
    }
}

#[tokio::test]
async fn a_dormant_instance_touches_nothing_until_configuration_is_committed() {
    let runtime = VectorRuntime::unconfigured();
    let connector = Arc::new(CountingConnector::new());
    let supervisor = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(CountingSleeper::default()),
    );
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { supervisor.supervise(shutdown).await }
    });

    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        connector.pair_calls.load(Ordering::SeqCst),
        0,
        "a dormant instance must not build a provider"
    );
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 0);
    assert!(runtime.snapshot().is_none());
    assert!(!runtime.is_ready());
    assert!(runtime.require_ready().is_err());

    // The first committed settings save marks the runtime configured and starts
    // a new generation, exactly as the application's observer does.
    runtime.mark_configured(true);
    runtime.begin_generation();

    for _ in 0..200 {
        tokio::task::yield_now().await;
        if runtime.snapshot().is_some() {
            break;
        }
    }

    let published = runtime
        .snapshot()
        .expect("activation must publish a pair without a restart");
    assert!(runtime.is_ready());
    assert_eq!(published.generation, runtime.generation());
    assert!(connector.pair_calls.load(Ordering::SeqCst) >= 1);
    assert!(connector.attach_calls.load(Ordering::SeqCst) >= 1);

    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_configuration_change_makes_a_stale_completion_unable_to_reopen_readiness() {
    let runtime = configured_runtime();
    let connector = Arc::new(CountingConnector::new());
    let recovery = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(CountingSleeper::default()),
    );

    assert!(recovery.run_startup(&CancellationToken::new()).await);
    let stale_generation = runtime.generation();
    assert!(runtime.is_ready());

    // A committed settings save advances the generation and withdraws readiness
    // from the pair the old generation published.
    runtime.begin_generation();
    assert!(!runtime.is_ready());
    assert!(
        !runtime.restore_ready(stale_generation),
        "a stale generation must never reopen readiness"
    );

    assert!(recovery.run_startup(&CancellationToken::new()).await);
    let current = runtime.snapshot().expect("the new generation publishes");
    assert_ne!(current.generation, stale_generation);
    assert_eq!(current.generation, runtime.generation());
    assert!(runtime.is_ready());
}

#[tokio::test]
async fn cancelling_a_dormant_supervisor_ends_it_without_touching_a_dependency() {
    let runtime = VectorRuntime::unconfigured();
    let connector = Arc::new(CountingConnector::new());
    let supervisor = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(StepSleeper::new()),
    );
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { supervisor.supervise(shutdown).await }
    });

    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("cancellation must end the dormant supervisor promptly")
        .expect("the supervisor task must not panic");
    assert_eq!(connector.pair_calls.load(Ordering::SeqCst), 0);
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 0);
    assert!(runtime.snapshot().is_none());
}
