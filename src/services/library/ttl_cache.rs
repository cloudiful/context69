//! Generic short-lived cache used by the library dependency gates.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Short-lived process-wide cache for read-mostly dependency state.
///
/// The application process targets a single database, and every reader accepts
/// the value being stale for at most the configured TTL.
pub(super) struct TtlCache<T> {
    ttl: Duration,
    state: Mutex<Option<(Instant, T)>>,
}

impl<T: Clone> TtlCache<T> {
    pub(super) const fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            state: Mutex::new(None),
        }
    }

    pub(super) fn get(&self) -> Option<T> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (fetched_at, value) = state.as_ref()?;
        (fetched_at.elapsed() < self.ttl).then(|| value.clone())
    }

    pub(super) fn put(&self, value: T) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *state = Some((Instant::now(), value));
    }

    /// Drop the cached value so the next reader fetches a fresh one, e.g. after
    /// the underlying row changed in a way the cached value cannot represent.
    pub(super) fn invalidate(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *state = None;
    }

    /// Return the cached value, or run `fetch`, cache the result and return it.
    /// A failed fetch is never cached, so the next reader retries.
    pub(super) async fn get_or_fetch<F, Fut, E>(&self, fetch: F) -> std::result::Result<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<T, E>>,
    {
        if let Some(value) = self.get() {
            return Ok(value);
        }
        let value = fetch().await?;
        self.put(value.clone());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::TtlCache;

    #[test]
    fn ttl_cache_serves_values_until_they_expire() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_millis(100));

        assert_eq!(cache.get(), None);
        cache.put("closed".to_string());
        assert_eq!(cache.get().as_deref(), Some("closed"));

        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(cache.get(), None);
    }

    #[test]
    fn ttl_cache_replaces_the_previous_value() {
        let cache: TtlCache<u8> = TtlCache::new(Duration::from_secs(60));

        cache.put(1);
        cache.put(2);

        assert_eq!(cache.get(), Some(2));
    }

    #[test]
    fn ttl_cache_invalidate_drops_the_value_immediately() {
        let cache: TtlCache<u8> = TtlCache::new(Duration::from_secs(60));

        cache.invalidate();
        assert_eq!(cache.get(), None);

        cache.put(7);
        assert_eq!(cache.get(), Some(7));

        cache.invalidate();
        assert_eq!(cache.get(), None);
    }

    #[tokio::test]
    async fn ttl_cache_get_or_fetch_skips_the_fetch_until_the_value_expires() {
        let calls = AtomicUsize::new(0);
        let cache: TtlCache<u8> = TtlCache::new(Duration::from_millis(100));
        let fetch = || {
            calls.fetch_add(1, Ordering::Relaxed);
            async { Ok::<u8, anyhow::Error>(41) }
        };

        assert_eq!(cache.get_or_fetch(fetch).await.unwrap(), 41);
        assert_eq!(cache.get_or_fetch(fetch).await.unwrap(), 41);
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(cache.get_or_fetch(fetch).await.unwrap(), 41);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn ttl_cache_get_or_fetch_does_not_cache_errors() {
        let calls = AtomicUsize::new(0);
        let cache: TtlCache<u8> = TtlCache::new(Duration::from_secs(60));
        let fetch = || {
            calls.fetch_add(1, Ordering::Relaxed);
            async { Err::<u8, anyhow::Error>(anyhow::anyhow!("unavailable")) }
        };

        assert!(cache.get_or_fetch(fetch).await.is_err());
        assert!(cache.get_or_fetch(fetch).await.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(cache.get(), None);
    }
}
