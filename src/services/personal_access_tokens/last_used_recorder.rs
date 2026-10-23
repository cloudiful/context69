//! Bounded, best-effort recording of personal access token usage.
//!
//! Usage time is approximate telemetry, never an authorization decision. The
//! recorder therefore coalesces observations per token, writes them in one
//! guarded batch per interval, and keeps every failure away from the request
//! that produced the observation: a slow, failed, or full sink never delays or
//! fails authentication.
//!
//! Ownership is one-way: the service holds the recorder, the recorder owns the
//! worker's cancellation and handle, and the worker owns the receiver and the
//! database handle. The worker never holds the recorder, so the last owner
//! dropping the recorder stops the worker.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::db::Database;

/// How many observations may wait in the queue before new ones are dropped.
pub(crate) const QUEUE_CAPACITY: usize = 1024;

/// How many distinct token ids may wait for the next flush.
pub(crate) const PENDING_CAPACITY: usize = 1024;

/// How long the worker waits between two flushes.
pub(crate) const FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// How long one flush may take, including acquiring a pooled connection.
pub(crate) const SINK_TIMEOUT: Duration = Duration::from_secs(2);

/// The writer the worker flushes observations to.
#[async_trait::async_trait]
pub(crate) trait UsageSink: Send + Sync + 'static {
    /// Write the usage timestamp of the given tokens.
    async fn touch(&self, token_ids: &[Uuid]) -> Result<u64>;
}

/// The source of the worker's flush ticks.
#[async_trait::async_trait]
pub(crate) trait FlushClock: Send + 'static {
    /// Wait for the next flush tick.
    async fn tick(&mut self);
}

/// Why one flush did not write its observations.
#[derive(Debug, Clone)]
pub(crate) enum FailureReason {
    /// The writer refused the batch.
    Rejected(String),
    /// The writer did not answer within the flush deadline.
    TimedOut,
}

/// One flush that did not write its observations.
#[derive(Debug, Clone)]
pub(crate) struct FlushFailure {
    /// How many token observations the flush carried.
    pub(crate) token_count: usize,
    /// Why the flush did not write them.
    pub(crate) reason: FailureReason,
}

/// The operator-facing description of a failed flush.
///
/// It carries the size of the batch and never a token id, a hash, or a URL.
pub(crate) fn flush_failure_message(failure: &FlushFailure) -> String {
    match &failure.reason {
        FailureReason::Rejected(error) => format!(
            "personal access token usage telemetry was not written for {} token(s): {error}",
            failure.token_count
        ),
        FailureReason::TimedOut => format!(
            "personal access token usage telemetry was not written for {} token(s): the write \
             exceeded its deadline",
            failure.token_count
        ),
    }
}

/// The receiver of a failed flush.
pub(crate) trait FlushReporter: Send + Sync + 'static {
    /// Report one failed flush.
    fn report(&self, failure: &FlushFailure);
}

/// The production reporter: one warning per failed flush.
struct LoggingReporter;

impl FlushReporter for LoggingReporter {
    fn report(&self, failure: &FlushFailure) {
        warn!("{}", flush_failure_message(failure));
    }
}

/// The production sink: the guarded batch statement.
struct DatabaseUsageSink {
    db: Database,
}

#[async_trait::async_trait]
impl UsageSink for DatabaseUsageSink {
    async fn touch(&self, token_ids: &[Uuid]) -> Result<u64> {
        self.db
            .touch_personal_access_tokens_last_used(token_ids)
            .await
    }
}

/// The production clock: one tick per interval, missed ticks skipped.
pub(crate) struct IntervalClock {
    interval: tokio::time::Interval,
}

impl IntervalClock {
    pub(crate) fn new(period: Duration) -> Self {
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self { interval }
    }
}

#[async_trait::async_trait]
impl FlushClock for IntervalClock {
    async fn tick(&mut self) {
        self.interval.tick().await;
    }
}

/// The bounded sizes and deadlines one recorder runs with.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecorderSettings {
    /// How many observations may wait in the queue.
    pub(crate) queue_capacity: usize,
    /// How many distinct token ids may wait for the next flush.
    pub(crate) pending_capacity: usize,
    /// How long one flush may take.
    pub(crate) sink_timeout: Duration,
    /// How many observations one flush may move out of the queue.
    pub(crate) drain_budget: usize,
}

impl RecorderSettings {
    /// The sizes and deadlines the application runs with.
    pub(crate) fn production() -> Self {
        Self {
            queue_capacity: QUEUE_CAPACITY,
            pending_capacity: PENDING_CAPACITY,
            sink_timeout: SINK_TIMEOUT,
            drain_budget: QUEUE_CAPACITY,
        }
    }
}

/// The one recorder every clone of the token service shares.
pub struct LastUsedRecorder {
    sender: mpsc::Sender<Uuid>,
    cancel: CancellationToken,
    worker: Mutex<Option<JoinHandle<()>>>,
    dropped: Arc<AtomicU64>,
    drained: Arc<AtomicU64>,
}

impl LastUsedRecorder {
    /// Start the recorder for the application's database.
    ///
    /// The worker runs for as long as an owner of the returned recorder lives.
    /// Without a runtime there is no worker to spawn, so every observation is
    /// dropped instead of panicking.
    #[must_use]
    pub fn start(db: Database) -> Arc<Self> {
        let (recorder, _stopped) = Self::spawn(
            Arc::new(DatabaseUsageSink { db }),
            Box::new(IntervalClock::new(FLUSH_INTERVAL)),
            Arc::new(LoggingReporter),
            RecorderSettings::production(),
        );
        recorder
    }

    /// Record one usage observation.
    ///
    /// The call never awaits, never locks, and never fails: a full or closed
    /// queue drops the observation and counts it.
    pub fn record(&self, token_id: Uuid) {
        if self.sender.try_send(token_id).is_err() {
            count_one(&self.dropped);
        }
    }

    /// How many observations were dropped instead of recorded.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// How many observations the worker moved out of the queue into a batch.
    #[must_use]
    pub fn drained(&self) -> u64 {
        self.drained.load(Ordering::Relaxed)
    }

    /// Start a recorder with an injected sink, clock, reporter, and settings.
    #[cfg(test)]
    pub(crate) fn start_with(
        sink: Arc<dyn UsageSink>,
        clock: Box<dyn FlushClock>,
        reporter: Arc<dyn FlushReporter>,
        settings: RecorderSettings,
    ) -> (Arc<Self>, watch::Receiver<bool>) {
        Self::spawn(sink, clock, reporter, settings)
    }

    /// Stop the worker while the recorder stays alive.
    ///
    /// The worker is cancelled and joined, so its queue is closed afterwards and
    /// every later observation is dropped instead of written.
    #[cfg(test)]
    pub(crate) async fn stop_worker(&self) {
        self.cancel.cancel();
        let handle = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    fn spawn(
        sink: Arc<dyn UsageSink>,
        clock: Box<dyn FlushClock>,
        reporter: Arc<dyn FlushReporter>,
        settings: RecorderSettings,
    ) -> (Arc<Self>, watch::Receiver<bool>) {
        let (sender, receiver) = mpsc::channel(settings.queue_capacity);
        let cancel = CancellationToken::new();
        let dropped = Arc::new(AtomicU64::new(0));
        let drained = Arc::new(AtomicU64::new(0));
        let (stopped, stopped_receiver) = watch::channel(false);
        let worker = Worker {
            receiver,
            sink,
            clock,
            reporter,
            cancel: cancel.clone(),
            pending_capacity: settings.pending_capacity,
            drain_budget: settings.drain_budget,
            sink_timeout: settings.sink_timeout,
            dropped: dropped.clone(),
            drained: drained.clone(),
        };
        // The guard is built before the task exists, so a worker that is
        // cancelled before its first poll still reports that it stopped.
        let guard = StopGuard { stopped };
        let handle = tokio::runtime::Handle::try_current().ok().map(|handle| {
            handle.spawn(async move {
                let _guard = guard;
                worker.run().await;
            })
        });
        (
            Arc::new(Self {
                sender,
                cancel,
                worker: Mutex::new(handle),
                dropped,
                drained,
            }),
            stopped_receiver,
        )
    }
}

impl Drop for LastUsedRecorder {
    fn drop(&mut self) {
        self.cancel.cancel();
        let handle = self
            .worker
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }
}

/// Counts one observation without ever wrapping around.
fn count_one(counter: &AtomicU64) {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_add(1);
        match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}

/// The worker that drains the queue and flushes it.
struct Worker {
    receiver: mpsc::Receiver<Uuid>,
    sink: Arc<dyn UsageSink>,
    clock: Box<dyn FlushClock>,
    reporter: Arc<dyn FlushReporter>,
    cancel: CancellationToken,
    pending_capacity: usize,
    drain_budget: usize,
    sink_timeout: Duration,
    dropped: Arc<AtomicU64>,
    drained: Arc<AtomicU64>,
}

/// Reports that the worker stopped, however it stopped.
struct StopGuard {
    stopped: watch::Sender<bool>,
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        let _ = self.stopped.send(true);
    }
}

impl Worker {
    async fn run(self) {
        let Worker {
            mut receiver,
            sink,
            mut clock,
            reporter,
            cancel,
            pending_capacity,
            drain_budget,
            sink_timeout,
            dropped,
            drained,
            ..
        } = self;
        let mut pending: HashSet<Uuid> = HashSet::new();
        loop {
            // The cancellation is checked before every round, so a worker that
            // is asked to stop never starts another drain or flush.
            if cancel.is_cancelled() {
                break;
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                received = receiver.recv() => match received {
                    Some(token_id) => observe(&mut pending, token_id, pending_capacity, &dropped),
                    None => break,
                },
                _ = clock.tick() => {
                    if cancel.is_cancelled() {
                        break;
                    }
                    drain(
                        &mut receiver,
                        &mut pending,
                        pending_capacity,
                        drain_budget,
                        &dropped,
                        &drained,
                    );
                    if cancel.is_cancelled() {
                        break;
                    }
                    flush(
                        &mut pending,
                        sink.as_ref(),
                        reporter.as_ref(),
                        sink_timeout,
                    )
                    .await;
                }
            }
        }
    }
}

/// Coalesce one observation into the pending set.
fn observe(
    pending: &mut HashSet<Uuid>,
    token_id: Uuid,
    pending_capacity: usize,
    dropped: &AtomicU64,
) {
    if pending.len() >= pending_capacity && !pending.contains(&token_id) {
        count_one(dropped);
        return;
    }
    pending.insert(token_id);
}

/// Move queued observations into the pending set, up to the drain budget.
///
/// The budget is what keeps one flush bounded: a producer that keeps
/// replenishing the queue cannot hold the worker here, so the flush still
/// starts and the cancellation is still observed.
fn drain(
    receiver: &mut mpsc::Receiver<Uuid>,
    pending: &mut HashSet<Uuid>,
    pending_capacity: usize,
    budget: usize,
    dropped: &AtomicU64,
    drained: &AtomicU64,
) {
    for _ in 0..budget {
        match receiver.try_recv() {
            Ok(token_id) => {
                count_one(drained);
                observe(pending, token_id, pending_capacity, dropped);
            }
            Err(_) => break,
        }
    }
}

/// Write the pending observations as one batch.
async fn flush(
    pending: &mut HashSet<Uuid>,
    sink: &dyn UsageSink,
    reporter: &dyn FlushReporter,
    sink_timeout: Duration,
) {
    if pending.is_empty() {
        return;
    }
    let mut token_ids: Vec<Uuid> = pending.drain().collect();
    token_ids.sort_unstable();
    let token_count = token_ids.len();
    let outcome = tokio::time::timeout(sink_timeout, sink.touch(&token_ids)).await;
    match outcome {
        Ok(Ok(_updated)) => {}
        Ok(Err(error)) => reporter.report(&FlushFailure {
            token_count,
            reason: FailureReason::Rejected(error.to_string()),
        }),
        Err(_elapsed) => reporter.report(&FlushFailure {
            token_count,
            reason: FailureReason::TimedOut,
        }),
    }
}
