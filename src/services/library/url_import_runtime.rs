use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use rate_limiter::{LocalRateLimiter, RateLimitPolicy, RateLimiter, ValkeyRateLimiter};
use tokio::sync::Notify;
use tracing::warn;

use super::LibraryService;

pub(super) const URL_IMPORT_RATE_LIMIT_PREFIX: &str = "context69:library:url-import:rate:";
pub(super) const URL_IMPORT_LEASE_TTL_SECS: i64 = 120;
pub(super) const URL_IMPORT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct UrlImportRuntime {
    wakeup: Arc<Notify>,
    limiter: Option<Arc<dyn RateLimiter>>,
    worker_count: usize,
    started: Arc<AtomicBool>,
}

impl UrlImportRuntime {
    pub(super) async fn new(
        worker_count: usize,
        min_interval_ms: u64,
        valkey_url: Option<&str>,
    ) -> Result<Self> {
        let policy = RateLimitPolicy::min_interval(Duration::from_millis(min_interval_ms))
            .context("invalid URL import rate limit policy")?;
        let limiter = match valkey_url {
            Some(url) => {
                match ValkeyRateLimiter::new(url, URL_IMPORT_RATE_LIMIT_PREFIX, policy).await {
                    Ok(limiter) => Some(Arc::new(limiter) as Arc<dyn RateLimiter>),
                    Err(error) => {
                        warn!(
                            %error,
                            "shared URL import rate limiter unavailable; URL import jobs remain queued"
                        );
                        None
                    }
                }
            }
            None => Some(Arc::new(
                LocalRateLimiter::new(policy)
                    .context("failed to initialize local URL import rate limiter")?,
            ) as Arc<dyn RateLimiter>),
        };

        Ok(Self {
            wakeup: Arc::new(Notify::new()),
            limiter,
            worker_count,
            started: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn limiter(&self) -> Option<Arc<dyn RateLimiter>> {
        self.limiter.clone()
    }

    pub(super) fn notify(&self) {
        self.wakeup.notify_waiters();
    }

    pub(super) async fn wait_for_work(&self) {
        self.wakeup.notified().await;
    }

    pub(super) fn poll_interval() -> Duration {
        WORKER_POLL_INTERVAL
    }

    pub(super) fn start_workers(&self, service: LibraryService) {
        if self.limiter.is_none() {
            return;
        }
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }

        for worker_id in 0..self.worker_count {
            let service = service.clone();
            let runtime = self.clone();
            tokio::spawn(async move {
                service.run_url_import_worker(runtime, worker_id).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UrlImportRuntime;

    #[tokio::test]
    async fn invalid_shared_limiter_does_not_fallback_to_local() {
        let runtime = UrlImportRuntime::new(2, 1000, Some("not-a-valkey-url"))
            .await
            .expect("runtime construction should keep queued work on limiter failure");

        assert!(runtime.limiter().is_none());
    }
}
