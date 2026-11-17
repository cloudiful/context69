use std::{future::Future, sync::Arc};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(super) struct ParkState {
    shutdown: CancellationToken,
    parked: watch::Sender<bool>,
}

tokio::task_local! {
    static CLAIM: Arc<ParkState>;
}

impl ParkState {
    pub(super) fn new(shutdown: CancellationToken) -> Arc<Self> {
        let (parked, _) = watch::channel(false);
        Arc::new(Self { shutdown, parked })
    }

    pub(super) async fn wait_parked(&self) {
        let mut parked = self.parked.subscribe();
        while !*parked.borrow_and_update() {
            if parked.changed().await.is_err() {
                return;
            }
        }
    }
}

pub(super) async fn scope<F: Future>(state: Arc<ParkState>, work: F) -> F::Output {
    CLAIM.scope(state, work).await
}

pub(super) async fn checkpoint() {
    let state = CLAIM.try_with(Arc::clone).ok();
    if let Some(state) = state {
        state.shutdown.cancelled().await;
        state.parked.send_replace(true);
        std::future::pending::<()>().await;
    } else {
        std::future::pending::<()>().await;
    }
}

pub(super) async fn interruptible<F: Future>(future: F) -> F::Output {
    tokio::select! {
        biased;
        () = checkpoint() => unreachable!(),
        value = future => value,
    }
}

pub(super) async fn park_if_shutdown() {
    if CLAIM
        .try_with(|state| state.shutdown.is_cancelled())
        .unwrap_or(false)
    {
        checkpoint().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn checkpoint_latches_and_never_returns() {
        let shutdown = CancellationToken::new();
        let state = ParkState::new(shutdown.clone());
        let work = tokio::spawn(scope(state.clone(), async {
            interruptible(std::future::pending::<()>()).await;
            panic!("park returned")
        }));
        shutdown.cancel();
        state.wait_parked().await;
        state.wait_parked().await;
        assert!(!work.is_finished());
        work.abort();
        assert!(work.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn shutdown_alone_does_not_park_noncheckpoint_work() {
        let shutdown = CancellationToken::new();
        let state = ParkState::new(shutdown.clone());
        let worker = tokio::spawn(scope(state.clone(), std::future::pending::<()>()));
        shutdown.cancel();
        tokio::task::yield_now().await;
        assert!(!*state.parked.borrow());
        assert!(!worker.is_finished());
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn parked_state_is_scoped_to_one_claim() {
        let shutdown = CancellationToken::new();
        let first = ParkState::new(shutdown.clone());
        let second = ParkState::new(CancellationToken::new());
        let worker = tokio::spawn(scope(
            first.clone(),
            interruptible(std::future::pending::<()>()),
        ));
        shutdown.cancel();
        first.wait_parked().await;
        assert!(!*second.parked.borrow());
        assert_eq!(scope(second, interruptible(async { 4 })).await, 4);
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn normal_completion_and_unscoped_work_continue() {
        let state = ParkState::new(CancellationToken::new());
        assert_eq!(scope(state, interruptible(async { 7 })).await, 7);
        assert_eq!(interruptible(async { 9 }).await, 9);
    }
}
