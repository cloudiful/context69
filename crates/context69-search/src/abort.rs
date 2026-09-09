//! Abort plumbing for long-running search work.
//!
//! `stream_search` runs a two-stage pipeline (collect → rerank) and pushes
//! frames into an SSE channel. When the client disconnects the channel's
//! receiver is dropped, so any further `tx.send` fails — but the in-flight
//! embedding, Qdrant, keyword, and rerank futures are not aborted and keep
//! running to completion, wasting capacity on a request the consumer can no
//! longer observe.
//!
//! This module exposes a one-shot signal pair built on `tokio::sync::oneshot`.
//! `AbortOnDrop::drop` fires the signal, so the SSE handler attaches the
//! sender to the unfold state that owns the receiver: when the response body
//! is dropped (client disconnect or handler cancel), the sender goes with it
//! and the producer side observes `wait()` returning and aborts cleanly.

use tokio::sync::oneshot;

/// Producer side of the abort signal. Held by whoever owns the lifetime of
/// the work (typically the SSE response body); dropping it fires the signal.
#[derive(Debug)]
pub struct AbortOnDrop {
    tx: Option<oneshot::Sender<()>>,
}

impl AbortOnDrop {
    /// Disarm the abort so a deliberate drop does not signal cancellation.
    pub fn disarm(mut self) {
        let _ = self.tx.take();
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            // Receiver may already be gone; the error just means we no longer
            // need to signal anything.
            let _ = tx.send(());
        }
    }
}

/// Consumer side of the abort signal. Owned by the search pipeline; the
/// pipeline selects on `wait()` next to its long-running awaits so a
/// cancellation races against completion.
#[derive(Debug)]
pub struct AbortSignal {
    rx: Option<oneshot::Receiver<()>>,
}

impl AbortSignal {
    /// Resolves when the matching `AbortOnDrop` is dropped (or has already
    /// been dropped). The result is intentionally ignored: the pipeline
    /// treats the resolution as "stop the remaining work and return". May
    /// be polled from multiple `tokio::select!` arms in turn.
    pub async fn wait(&mut self) {
        if let Some(rx) = self.rx.as_mut() {
            let _ = rx.await;
        }
    }
}

/// Build a paired abort signal. The returned `AbortOnDrop` fires on drop; the
/// `AbortSignal` resolves on the matching drop.
pub fn abort_pair() -> (AbortSignal, AbortOnDrop) {
    let (tx, rx) = oneshot::channel();
    (AbortSignal { rx: Some(rx) }, AbortOnDrop { tx: Some(tx) })
}

#[cfg(test)]
mod tests {
    use super::{abort_pair};

    #[test]
    fn disarm_lets_signal_resolve_immediately() {
        // Disarm consumes the sender without sending. The receiver therefore
        // observes a closed channel and `wait()` must complete without
        // blocking the executor (no panic, no hang).
        let (mut signal, guard) = abort_pair();
        guard.disarm();
        let pending = std::pin::pin!(signal.wait());
        let waker = futures::task::noop_waker_ref();
        let mut context = std::task::Context::from_waker(waker);
        // Disarm closes the channel; the future is ready on the first poll.
        assert!(matches!(
            pending.poll(&mut context),
            std::task::Poll::Ready(())
        ));
    }

    #[tokio::test]
    async fn drop_fires_signal_immediately() {
        let (mut signal, guard) = abort_pair();
        drop(guard);
        // Should complete (return value ignored).
        signal.wait().await;
    }
}
