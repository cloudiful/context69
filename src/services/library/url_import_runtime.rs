use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use rate_limiter::{LocalRateLimiter, RateLimitPolicy, RateLimiter, ValkeyRateLimiter};
use tracing::warn;

pub(super) const URL_IMPORT_RATE_LIMIT_PREFIX: &str = "context69:library:url-import:rate:";
#[derive(Clone)]
pub(super) struct UrlImportRuntime {
    limiter: Option<Arc<dyn RateLimiter>>,
}

impl UrlImportRuntime {
    pub(super) async fn new(
        _worker_count: usize,
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

        Ok(Self { limiter })
    }

    pub(super) fn limiter(&self) -> Option<Arc<dyn RateLimiter>> {
        self.limiter.clone()
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
