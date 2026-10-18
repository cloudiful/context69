//! Deterministic recovery sequencing: retry budget, permanent pause,
//! cancellation, probe-driven readiness, and generation fencing.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    embedding::{EmbeddingIdentity, EmbeddingProvider},
    qdrant_index::QdrantIndex,
    vector_runtime::{
        VectorFailureClass, VectorRuntime, VectorSnapshot,
        recovery::{
            RecoveryObserver, RecoveryPolicy, Sleeper, VectorConnector, VectorPair, VectorRecovery,
        },
    },
};

struct TestProvider;

#[async_trait]
impl EmbeddingProvider for TestProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
    }
}

struct FakeConnector {
    attach_results: Mutex<VecDeque<Result<(), &'static str>>>,
    probe_results: Mutex<VecDeque<Result<(), &'static str>>>,
    attach_calls: AtomicUsize,
    probe_calls: AtomicUsize,
    fingerprint: Mutex<String>,
}

impl FakeConnector {
    fn new() -> Self {
        Self {
            attach_results: Mutex::new(VecDeque::new()),
            probe_results: Mutex::new(VecDeque::new()),
            attach_calls: AtomicUsize::new(0),
            probe_calls: AtomicUsize::new(0),
            fingerprint: Mutex::new("fingerprint-a".to_string()),
        }
    }

    fn fail_attach(&self, times: usize, message: &'static str) {
        let mut results = self.attach_results.lock().expect("attach queue");
        for _ in 0..times {
            results.push_back(Err(message));
        }
    }

    fn fail_probe(&self, times: usize, message: &'static str) {
        let mut results = self.probe_results.lock().expect("probe queue");
        for _ in 0..times {
            results.push_back(Err(message));
        }
    }
}

#[async_trait]
impl VectorConnector for FakeConnector {
    async fn pair(&self) -> Result<VectorPair> {
        Ok(VectorPair {
            embedding: Arc::new(TestProvider),
            identity: EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
            collection: "test".to_string(),
        })
    }

    async fn attach(&self, _pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        self.attach_calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .attach_results
            .lock()
            .expect("attach queue")
            .pop_front()
            .unwrap_or(Ok(()));
        match outcome {
            Ok(()) => Ok(Arc::new(QdrantIndex::for_test("test", 8))),
            Err(message) => Err(anyhow::anyhow!("{message}")),
        }
    }

    async fn probe(&self, _snapshot: &VectorSnapshot) -> Result<()> {
        self.probe_calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .probe_results
            .lock()
            .expect("probe queue")
            .pop_front()
            .unwrap_or(Ok(()));
        match outcome {
            Ok(()) => Ok(()),
            Err(message) => Err(anyhow::anyhow!("{message}")),
        }
    }

    fn configuration_fingerprint(&self) -> String {
        self.fingerprint.lock().expect("fingerprint").clone()
    }
}

#[derive(Default)]
struct RecordingSleeper {
    total: Mutex<Duration>,
    sleeps: AtomicUsize,
}

#[async_trait]
impl Sleeper for RecordingSleeper {
    async fn sleep(&self, duration: Duration) {
        *self.total.lock().expect("total") += duration;
        self.sleeps.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
    }
}

/// Blocks each sleep until the test explicitly advances it.
struct StepSleeper {
    notify: Notify,
}

impl StepSleeper {
    fn new() -> Self {
        Self {
            notify: Notify::new(),
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

#[derive(Debug)]
enum Event {
    Published,
    Failed,
    ProbeFailed,
    ProbeRecovered,
}

struct TestObserver(mpsc::UnboundedSender<Event>);

#[async_trait]
impl RecoveryObserver for TestObserver {
    async fn published(&self, _snapshot: &VectorSnapshot) {
        let _ = self.0.send(Event::Published);
    }

    async fn failed(&self, _generation: u64, _class: VectorFailureClass, _message: &str) {
        let _ = self.0.send(Event::Failed);
    }

    async fn probe_failed(&self, _snapshot: &VectorSnapshot, _message: &str) {
        let _ = self.0.send(Event::ProbeFailed);
    }

    async fn probe_recovered(&self, _snapshot: &VectorSnapshot) {
        let _ = self.0.send(Event::ProbeRecovered);
    }
}

fn engine(
    runtime: VectorRuntime,
    connector: Arc<FakeConnector>,
    sleeper: Arc<dyn Sleeper>,
) -> VectorRecovery {
    VectorRecovery::new(runtime, connector, sleeper)
}

#[tokio::test]
async fn recovery_diagnostics_redact_endpoint_urls() {
    let message = super::sanitize_message(
        "connect https://user:secret@qdrant.example:6334/collections failed after 2s",
    );
    assert!(!message.contains("user:secret"), "{message}");
    assert!(!message.contains("qdrant.example"), "{message}");
    assert!(message.contains("failed"), "{message}");
}

#[tokio::test]
async fn startup_retries_transient_failures_then_publishes() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    connector.fail_attach(2, "connection refused");
    let recovery = engine(
        runtime.clone(),
        connector.clone(),
        Arc::new(RecordingSleeper::default()),
    );

    assert!(recovery.run_startup(&CancellationToken::new()).await);
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 3);
    assert!(runtime.is_ready());
    assert!(runtime.snapshot().is_some());
}

#[tokio::test]
async fn startup_budget_is_bounded_and_reports_the_retry_sequence() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    connector.fail_attach(100, "connection refused");
    let sleeper = Arc::new(RecordingSleeper::default());
    let policy = RecoveryPolicy {
        startup_budget: Duration::from_secs(30),
        ..RecoveryPolicy::default()
    };
    let recovery = engine(runtime, connector.clone(), sleeper.clone()).with_policy(policy);

    assert!(!recovery.run_startup(&CancellationToken::new()).await);
    let total = *sleeper.total.lock().expect("total");
    assert!(
        total <= Duration::from_secs(30),
        "the startup budget must bound the total sleep, got {total:?}"
    );
    assert!(sleeper.sleeps.load(Ordering::SeqCst) <= 5);
    assert!(connector.attach_calls.load(Ordering::SeqCst) <= 6);
}

#[tokio::test]
async fn a_permanent_configuration_failure_pauses_the_startup_retry() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    connector.fail_attach(1, "dimension mismatch expected 8 found 16");
    let recovery = engine(
        runtime,
        connector.clone(),
        Arc::new(RecordingSleeper::default()),
    );

    assert!(!recovery.run_startup(&CancellationToken::new()).await);
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_cancelled_startup_interrupts_the_backoff_promptly() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    connector.fail_attach(100, "connection refused");
    let recovery = VectorRecovery::new(
        runtime,
        connector,
        Arc::new(crate::vector_runtime::recovery::TokioSleeper),
    );
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        token.cancel();
    });

    let started = std::time::Instant::now();
    let run = tokio::spawn(async move { recovery.run_startup(&shutdown).await });
    let connected = run.await.expect("startup task");
    assert!(!connected);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation must interrupt the backoff, took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_failed_probe_marks_not_ready_and_retains_the_snapshot() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    let sleeper = Arc::new(StepSleeper::new());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let recovery = engine(runtime.clone(), connector.clone(), sleeper.clone())
        .with_observer(Arc::new(TestObserver(tx)));

    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { recovery.supervise(shutdown).await }
    });

    // First sleep happens only after a published, ready pair. The supervisor
    // attempts the bootstrap before its first sleep, so wait for Published.
    assert!(matches!(rx.recv().await, Some(Event::Published)));
    let snapshot = runtime.snapshot().expect("snapshot after publish");
    connector.fail_probe(1, "transport error");
    sleeper.step();
    assert!(matches!(rx.recv().await, Some(Event::ProbeFailed)));
    assert!(!runtime.is_ready());
    let retained = runtime.snapshot().expect("probe failure keeps the client");
    assert_eq!(retained.generation, snapshot.generation);

    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_successful_probe_restores_the_same_generation() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    let sleeper = Arc::new(StepSleeper::new());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let recovery = engine(runtime.clone(), connector.clone(), sleeper.clone())
        .with_observer(Arc::new(TestObserver(tx)));

    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { recovery.supervise(shutdown).await }
    });

    assert!(matches!(rx.recv().await, Some(Event::Published)));
    let snapshot = runtime.snapshot().expect("snapshot after publish");
    sleeper.step();
    assert!(matches!(rx.recv().await, Some(Event::ProbeRecovered)));
    assert!(runtime.is_ready());
    assert_eq!(
        runtime.snapshot().expect("snapshot retained").generation,
        snapshot.generation
    );

    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_stale_attempt_cannot_publish_after_a_newer_generation() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    let sleeper = Arc::new(StepSleeper::new());
    let recovery = engine(runtime.clone(), connector.clone(), sleeper.clone());
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { recovery.supervise(shutdown).await }
    });

    // Let the first attempt run, then supersede it before it can publish.
    tokio::task::yield_now().await;
    let newer = runtime.begin_generation();
    sleeper.step();
    tokio::task::yield_now().await;

    // The supervisor retries under the new generation and publishes only that.
    for _ in 0..10 {
        tokio::task::yield_now().await;
        if let Some(snapshot) = runtime.snapshot() {
            assert_eq!(snapshot.generation, newer);
            break;
        }
    }
    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_permanent_probe_failure_pauses_until_the_fingerprint_changes() {
    let runtime = VectorRuntime::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let connector = Arc::new(FakeConnector::new());
    let sleeper = Arc::new(RecordingSleeper::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let policy = RecoveryPolicy {
        probe_interval: Duration::from_secs(10),
        ..RecoveryPolicy::default()
    };
    let recovery = engine(runtime.clone(), connector.clone(), sleeper.clone())
        .with_observer(Arc::new(TestObserver(tx)))
        .with_policy(policy);
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { recovery.supervise(shutdown).await }
    });

    assert!(matches!(rx.recv().await, Some(Event::Published)));
    connector.fail_probe(1, "Unauthenticated invalid api key");
    // The immediate sleep ends at once; the probe then fails permanently.
    assert!(matches!(rx.recv().await, Some(Event::ProbeFailed)));
    let calls_after_pause = connector.probe_calls.load(Ordering::SeqCst);

    // The paused loop sleeps without reconnecting or probing.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        connector.probe_calls.load(Ordering::SeqCst),
        calls_after_pause
    );
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 1);

    // A new fingerprint (settings generation) allows a fresh attempt.
    *connector.fingerprint.lock().expect("fingerprint") = "fingerprint-b".to_string();
    runtime.begin_generation();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(5)).await;
        if connector.attach_calls.load(Ordering::SeqCst) > 1 {
            break;
        }
    }
    assert!(connector.attach_calls.load(Ordering::SeqCst) > 1);

    shutdown.cancel();
    let _ = handle.await;
}
