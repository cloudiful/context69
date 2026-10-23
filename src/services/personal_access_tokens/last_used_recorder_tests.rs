//! The bounded usage recorder, proven against a controlled sink and clock.
//!
//! Every case drives the worker by hand: the sink is a test server that
//! decides when a batch is answered, and the clock is a manual tick channel.
//! Nothing here waits for a wall-clock minute, and nothing touches a database.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

use super::last_used_recorder::{
    FLUSH_INTERVAL, FailureReason, FlushClock, FlushFailure, FlushReporter, IntervalClock,
    LastUsedRecorder, PENDING_CAPACITY, QUEUE_CAPACITY, RecorderSettings, SINK_TIMEOUT, UsageSink,
    flush_failure_message,
};

/// One batch the worker offered to the sink.
struct Call {
    token_ids: Vec<Uuid>,
    respond: oneshot::Sender<Result<u64>>,
}

/// The sink the test answers by hand.
struct FakeSink {
    calls: mpsc::UnboundedSender<Call>,
}

#[async_trait::async_trait]
impl UsageSink for FakeSink {
    async fn touch(&self, token_ids: &[Uuid]) -> Result<u64> {
        let (respond, answered) = oneshot::channel();
        self.calls
            .send(Call {
                token_ids: token_ids.to_vec(),
                respond,
            })
            .map_err(|_| anyhow!("the sink server is gone"))?;
        match answered.await {
            Ok(result) => result,
            Err(_) => Err(anyhow!("the sink call was abandoned")),
        }
    }
}

/// The clock the test ticks by hand.
struct ManualClock {
    ticks: mpsc::Receiver<()>,
}

#[async_trait::async_trait]
impl FlushClock for ManualClock {
    async fn tick(&mut self) {
        if self.ticks.recv().await.is_none() {
            // A closed tick channel must park the worker instead of spinning.
            std::future::pending::<()>().await;
        }
    }
}

/// The reporter the test reads instead of the log.
#[derive(Default)]
struct RecordingReporter {
    failures: Mutex<Vec<FlushFailure>>,
}

impl FlushReporter for RecordingReporter {
    fn report(&self, failure: &FlushFailure) {
        self.failures.lock().unwrap().push(failure.clone());
    }
}

/// The harness one case drives.
struct Harness {
    recorder: Option<Arc<LastUsedRecorder>>,
    calls: mpsc::UnboundedReceiver<Call>,
    ticks: mpsc::Sender<()>,
    reporter: Arc<RecordingReporter>,
    stopped: watch::Receiver<bool>,
}

impl Harness {
    fn start(settings: RecorderSettings) -> Self {
        let (calls, call_receiver) = mpsc::unbounded_channel();
        let (tick_sender, tick_receiver) = mpsc::channel(1);
        let reporter = Arc::new(RecordingReporter::default());
        let (recorder, stopped) = LastUsedRecorder::start_with(
            Arc::new(FakeSink { calls }),
            Box::new(ManualClock {
                ticks: tick_receiver,
            }),
            reporter.clone(),
            settings,
        );
        Self {
            recorder: Some(recorder),
            calls: call_receiver,
            ticks: tick_sender,
            reporter,
            stopped,
        }
    }

    /// The settings the tests run with: the production sizes, a short deadline.
    fn settings() -> RecorderSettings {
        RecorderSettings {
            queue_capacity: QUEUE_CAPACITY,
            pending_capacity: PENDING_CAPACITY,
            sink_timeout: Duration::from_millis(200),
            drain_budget: QUEUE_CAPACITY,
        }
    }

    fn recorder(&self) -> &Arc<LastUsedRecorder> {
        self.recorder
            .as_ref()
            .expect("the recorder is still owned by the harness")
    }

    /// Give up the harness's ownership of the recorder.
    fn release_recorder(&mut self) -> Arc<LastUsedRecorder> {
        self.recorder
            .take()
            .expect("the recorder is still owned by the harness")
    }

    /// Ask the worker for one flush.
    fn tick(&self) {
        // A tick that nobody is ready for is dropped, exactly like a missed
        // interval tick.
        let _ = self.ticks.try_send(());
    }

    /// Wait for the batch the worker offered to the sink.
    async fn next_call(&mut self) -> Call {
        tokio::time::timeout(Duration::from_secs(5), self.calls.recv())
            .await
            .expect("the worker offers a batch")
            .expect("the sink server is open")
    }

    /// Assert that no batch is offered.
    async fn assert_no_call(&mut self) {
        let offered = tokio::time::timeout(Duration::from_millis(200), self.calls.recv()).await;
        match offered {
            Err(_) => {}
            Ok(Some(call)) => {
                let ids: Vec<String> = call.token_ids.iter().map(Uuid::to_string).collect();
                panic!("the worker must not write without an observation, wrote {ids:?}");
            }
            // A closed sink server means the worker is gone, which is the
            // strongest form of "no batch".
            Ok(None) => {}
        }
    }

    fn failures(&self) -> Vec<FlushFailure> {
        self.reporter.failures.lock().unwrap().clone()
    }

    /// Wait until the reporter recorded at least `expected` failed flushes.
    ///
    /// The reporter runs in the worker, so its report lands one step after the
    /// sink answered. The wait is bounded and never relaxes the count.
    async fn await_failure_count(&self, expected: usize) -> Vec<FlushFailure> {
        for _ in 0..200 {
            let failures = self.failures();
            if failures.len() >= expected {
                return failures;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.failures()
    }
}

/// The architecture the issue fixed.
#[test]
fn the_recorder_keeps_its_bounded_sizes_and_deadlines() {
    assert_eq!(QUEUE_CAPACITY, 1024);
    assert_eq!(PENDING_CAPACITY, 1024);
    assert_eq!(FLUSH_INTERVAL, Duration::from_secs(60));
    assert_eq!(SINK_TIMEOUT, Duration::from_secs(2));
}

#[tokio::test]
async fn a_burst_of_one_token_reaches_the_sink_once() {
    let mut harness = Harness::start(Harness::settings());
    let token_id = Uuid::new_v4();
    for _ in 0..100 {
        harness.recorder().record(token_id);
    }

    harness.tick();
    let call = harness.next_call().await;
    assert_eq!(call.token_ids, vec![token_id]);
    let _ = call.respond.send(Ok(1));

    harness.assert_no_call().await;
}

#[tokio::test]
async fn distinct_tokens_reach_the_sink_as_one_sorted_batch() {
    let mut harness = Harness::start(Harness::settings());
    let mut expected: Vec<Uuid> = (0..8).map(|_| Uuid::new_v4()).collect();
    expected.sort_unstable();
    for token_id in &expected {
        harness.recorder().record(*token_id);
    }

    harness.tick();
    let call = harness.next_call().await;
    assert_eq!(call.token_ids, expected);
    let _ = call.respond.send(Ok(expected.len() as u64));
}

#[tokio::test]
async fn an_idle_worker_never_writes() {
    let mut harness = Harness::start(Harness::settings());

    harness.tick();
    harness.assert_no_call().await;
    assert!(harness.failures().is_empty());
}

#[tokio::test]
async fn a_full_queue_drops_observations_without_blocking_the_caller() {
    let mut harness = Harness::start(Harness::settings());
    let blocked = Uuid::new_v4();
    harness.recorder().record(blocked);
    harness.tick();
    let held = harness.next_call().await;
    assert_eq!(held.token_ids, vec![blocked]);

    // The worker is inside the sink, so the queue fills up while it waits.
    for _ in 0..(QUEUE_CAPACITY + 16) {
        harness.recorder().record(Uuid::new_v4());
    }
    assert!(
        harness.recorder().dropped() > 0,
        "a full queue drops the observations it cannot hold"
    );

    // The caller is never held up by the blocked sink: no second batch is
    // written while the first one is still in flight.
    harness.assert_no_call().await;
    let _ = held.respond.send(Ok(1));
}

#[tokio::test]
async fn one_flush_is_in_flight_at_a_time() {
    let mut harness = Harness::start(Harness::settings());
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let held = harness.next_call().await;

    harness.tick();
    harness.assert_no_call().await;

    let _ = held.respond.send(Ok(1));
}

#[tokio::test]
async fn overlapping_ticks_are_skipped_instead_of_queued() {
    let mut harness = Harness::start(Harness::settings());
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let held = harness.next_call().await;

    for _ in 0..5 {
        harness.tick();
    }
    harness.assert_no_call().await;

    let _ = held.respond.send(Ok(1));
    // At most one skipped tick survives, and it carries no observation of its
    // own, so it never writes a second batch.
    tokio::time::sleep(Duration::from_millis(100)).await;
    harness.assert_no_call().await;
}

#[tokio::test]
async fn a_flush_that_exceeds_its_deadline_is_dropped_and_not_retried() {
    let mut harness = Harness::start(Harness::settings());
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let held = harness.next_call().await;

    // The sink never answers, so the flush reaches its deadline.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        held.respond.is_closed(),
        "the worker gives the abandoned batch up at its deadline"
    );
    let failures = harness.await_failure_count(1).await;
    assert_eq!(failures.len(), 1);
    assert!(matches!(failures[0].reason, FailureReason::TimedOut));

    // The failed batch is not retried on its own.
    harness.assert_no_call().await;

    // The next window writes the observations that follow it.
    let token_id = Uuid::new_v4();
    harness.recorder().record(token_id);
    harness.tick();
    let recovered = harness.next_call().await;
    assert_eq!(recovered.token_ids, vec![token_id]);
    let _ = recovered.respond.send(Ok(1));
    assert_eq!(harness.failures().len(), 1);
}

#[tokio::test]
async fn a_refused_flush_is_harmless_and_the_next_window_writes_again() {
    let mut harness = Harness::start(Harness::settings());
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let refused = harness.next_call().await;
    let _ = refused.respond.send(Err(anyhow!("the write was refused")));

    let failures = harness.await_failure_count(1).await;
    assert_eq!(failures.len(), 1);
    assert!(matches!(failures[0].reason, FailureReason::Rejected(_)));

    let token_id = Uuid::new_v4();
    harness.recorder().record(token_id);
    harness.tick();
    let recovered = harness.next_call().await;
    assert_eq!(recovered.token_ids, vec![token_id]);
    let _ = recovered.respond.send(Ok(1));
}

#[tokio::test]
async fn each_failed_window_is_reported_once_and_a_healthy_window_adds_none() {
    let mut harness = Harness::start(Harness::settings());

    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let refused = harness.next_call().await;
    let _ = refused.respond.send(Err(anyhow!("the write was refused")));
    assert_eq!(harness.await_failure_count(1).await.len(), 1);

    // The next window fails as well, and reports exactly one more.
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let refused = harness.next_call().await;
    let _ = refused.respond.send(Err(anyhow!("the write was refused")));
    assert_eq!(harness.await_failure_count(2).await.len(), 2);

    // A healthy window adds no report at all.
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let healthy = harness.next_call().await;
    let _ = healthy.respond.send(Ok(1));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(harness.failures().len(), 2);
}

#[tokio::test]
async fn a_failed_flush_reports_the_batch_size_and_never_a_token() {
    let mut harness = Harness::start(Harness::settings());
    let token_id = Uuid::new_v4();
    harness.recorder().record(token_id);
    harness.tick();
    let refused = harness.next_call().await;
    let _ = refused.respond.send(Err(anyhow!("the write was refused")));

    let failures = harness.await_failure_count(1).await;
    assert_eq!(failures.len(), 1);
    let message = flush_failure_message(&failures[0]);
    assert!(message.contains("1 token(s)"), "{message}");
    assert!(!message.contains(&token_id.to_string()), "{message}");
    assert!(!message.contains("ctx_pat_"), "{message}");
}

#[tokio::test]
async fn a_clone_keeps_the_worker_alive_and_the_last_owner_stops_it() {
    let mut harness = Harness::start(Harness::settings());
    let clone = harness.recorder().clone();

    harness.recorder().record(Uuid::new_v4());
    drop(harness.release_recorder());
    harness.tick();
    let call = harness.next_call().await;
    let _ = call.respond.send(Ok(1));

    drop(clone);
    tokio::time::timeout(Duration::from_secs(5), harness.stopped.changed())
        .await
        .expect("the worker stops within its deadline")
        .expect("the worker reports that it stopped");
    assert!(*harness.stopped.borrow_and_update());

    harness.tick();
    harness.assert_no_call().await;
}

#[tokio::test]
async fn the_worker_does_not_outlive_its_last_owner() {
    let mut harness = Harness::start(Harness::settings());
    // The last owner gives the recorder up, so the worker is cancelled and
    // aborted even though it never served an observation.
    drop(harness.release_recorder());

    tokio::time::timeout(Duration::from_secs(5), harness.stopped.changed())
        .await
        .expect("the worker stops within its deadline")
        .expect("the worker reports that it stopped");
    assert!(*harness.stopped.borrow_and_update());

    harness.tick();
    harness.assert_no_call().await;
}

#[tokio::test]
async fn a_stopped_recorder_drops_its_observations_instead_of_failing() {
    let mut harness = Harness::start(Harness::settings());
    harness.recorder().stop_worker().await;
    // The worker is gone with its queue, so recording into it is harmless and
    // is counted as a drop.
    harness.recorder().record(Uuid::new_v4());
    assert!(
        harness.recorder().dropped() > 0,
        "a closed queue drops the observation instead of failing"
    );
    harness.tick();
    harness.assert_no_call().await;
}

/// The pending set is bounded, so a flood of distinct tokens cannot grow it
/// without limit.
///
/// The queue is deliberately larger than the pending set and the drain budget
/// is larger than the pending set too, so the pending guard is the only thing
/// that keeps the batch small: without it the batch would carry the whole
/// drain budget.
#[tokio::test]
async fn the_pending_set_is_bounded() {
    let settings = RecorderSettings {
        queue_capacity: PENDING_CAPACITY * 4,
        pending_capacity: PENDING_CAPACITY,
        sink_timeout: Duration::from_millis(200),
        drain_budget: PENDING_CAPACITY * 2,
    };
    let mut harness = Harness::start(settings);
    let blocked = Uuid::new_v4();
    harness.recorder().record(blocked);
    harness.tick();
    let held = harness.next_call().await;

    // More distinct tokens than the pending set can hold, all admitted into the
    // queue that is large enough to take them.
    for _ in 0..(PENDING_CAPACITY * 3) {
        harness.recorder().record(Uuid::new_v4());
    }

    let _ = held.respond.send(Ok(1));
    harness.tick();
    let call = harness.next_call().await;
    assert!(
        call.token_ids.len() <= PENDING_CAPACITY,
        "the pending set stays bounded, not the drain budget: {} ids",
        call.token_ids.len()
    );
    assert!(
        harness.recorder().dropped() > 0,
        "the observations the pending set could not take are dropped"
    );
    let unique: HashSet<Uuid> = call.token_ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        call.token_ids.len(),
        "a batch carries no duplicates"
    );
    let _ = call.respond.send(Ok(1));
}

/// One flush moves at most the drain budget out of the queue.
///
/// The queue holds far more than the budget, so a drain that waits for an
/// empty queue would move all of them, and a flush would never start while a
/// producer keeps the queue full.
#[tokio::test]
async fn one_flush_moves_at_most_the_drain_budget() {
    let settings = RecorderSettings {
        queue_capacity: PENDING_CAPACITY * 4,
        pending_capacity: PENDING_CAPACITY * 4,
        sink_timeout: Duration::from_millis(200),
        drain_budget: PENDING_CAPACITY,
    };
    let mut harness = Harness::start(settings);

    // The worker is busy in the sink, so the queue keeps what it receives.
    harness.recorder().record(Uuid::new_v4());
    harness.tick();
    let held = harness.next_call().await;
    for _ in 0..(PENDING_CAPACITY * 3) {
        harness.recorder().record(Uuid::new_v4());
    }

    let before = harness.recorder().drained();
    let _ = held.respond.send(Ok(1));
    harness.tick();
    let call = harness.next_call().await;
    let moved = harness.recorder().drained() - before;
    assert!(
        moved <= PENDING_CAPACITY as u64,
        "one flush moves at most the drain budget: {moved} observations"
    );
    let _ = call.respond.send(Ok(1));
}

/// A producer that keeps the queue full until it is told to stop.
///
/// The producer runs on its own operating-system thread, so it keeps
/// replenishing the queue while the worker is busy inside it. A producer that
/// shares the runtime's thread cannot outrun the worker, and the case would
/// prove nothing.
struct Producer {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Producer {
    fn start(recorder: Arc<LastUsedRecorder>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let ids: Vec<Uuid> = (0..64).map(|_| Uuid::new_v4()).collect();
                let mut index = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    recorder.record(ids[index % ids.len()]);
                    index += 1;
                }
            })
        };
        Self {
            stop,
            handle: Some(handle),
        }
    }

    /// Stop the producer and wait for it to finish.
    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("the producer stops");
        }
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        // A failing case must not leave the producer running.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A producer that never stops must not hold the worker.
///
/// The queue is full the whole time, so the worker has to leave the drain
/// behind its budget to reach the flush, and it has to notice the
/// cancellation while the producer is still running.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sustained_producer_still_reaches_a_flush_and_stops_the_worker() {
    let mut harness = Harness::start(Harness::settings());
    let recorder = harness.recorder().clone();
    let producer = Producer::start(recorder.clone());

    // Let the producer fill the queue before the flush is due.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        harness.recorder().dropped() > 0,
        "the producer outruns the queue while the worker is busy"
    );

    harness.tick();
    let call = tokio::time::timeout(Duration::from_secs(5), harness.next_call())
        .await
        .expect("a ready flush starts while the producer keeps the queue full");
    let _ = call.respond.send(Ok(1));

    // The worker is cancelled while the producer is still running.
    let started = std::time::Instant::now();
    recorder.stop_worker().await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "the worker stops within a bounded time: {elapsed:?}"
    );
    assert!(
        *harness.stopped.borrow_and_update(),
        "the worker reports that it stopped"
    );

    producer.stop();
}

/// The production clock waits a full period and skips the ticks it missed.
#[tokio::test]
async fn the_production_clock_waits_a_full_period_and_skips_missed_ticks() {
    let period = Duration::from_millis(40);
    let started = std::time::Instant::now();
    let mut clock = IntervalClock::new(period);

    clock.tick().await;
    let first = started.elapsed();
    assert!(
        first >= period,
        "the first tick waits a full period: {first:?}"
    );
    assert!(
        first < period * 3,
        "the first tick is not delayed further than one period: {first:?}"
    );

    // The consumer stalls for several periods.
    tokio::time::sleep(period * 5).await;

    let mut ticks = 0;
    let deadline = std::time::Instant::now() + period * 2;
    while std::time::Instant::now() < deadline {
        clock.tick().await;
        ticks += 1;
    }
    assert!(
        ticks <= 4,
        "the missed ticks are skipped, not replayed: {ticks} ticks in two periods"
    );
}
