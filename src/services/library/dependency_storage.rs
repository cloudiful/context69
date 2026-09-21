use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use opendal::ErrorKind;

use crate::domain_errors::DomainError;
use tokio::time::timeout;
use uuid::Uuid;

use super::dependency_errors::{
    is_configuration_error, is_s3_attempt_retryable, is_s3_transient_error,
};
use super::dependency_runtime::TtlCache;
use super::{LibraryDependency, LibraryService};
use crate::library_store::{DependencyGateRecord, DependencyGateTransition, LibraryStore};

const S3_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const S3_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const S3_RETRY_LIMIT: usize = 2;

/// Every S3 read/write re-checks the S3 gate before touching the backend, so the
/// `ORDER BY dependency_key` scan behind `list_dependency_gates` belongs off the
/// hot path. The cached record is dropped as soon as this process records a gate
/// transition, so a local state change is observed by the next operation; the TTL
/// only bounds how long a steady-state record is reused, which is how long
/// another instance's transition can take to be observed.
const S3_GATE_CACHE_TTL: Duration = Duration::from_secs(2);

static S3_GATE_CACHE: TtlCache<DependencyGateRecord> = TtlCache::new(S3_GATE_CACHE_TTL);

/// Drop the cached S3 gate record when the gate actually changed state. Serving a
/// stale `closed` record would defeat a gate that just opened, and serving a stale
/// `open` record would reject operations for up to the TTL after the gate healed.
pub(super) fn observe_s3_gate_transition(
    dependency: LibraryDependency,
    transition: Option<&DependencyGateTransition>,
) {
    if dependency.canonical() == LibraryDependency::S3
        && transition.is_some_and(|transition| transition.transitioned)
    {
        S3_GATE_CACHE.invalidate();
    }
}

/// Read the S3 gate through [`S3_GATE_CACHE`]: a fresh record is cached and a
/// missing row is not, so a startup race cannot pin an unavailable dependency.
async fn cached_s3_gate(store: &LibraryStore) -> Result<Option<DependencyGateRecord>> {
    if let Some(gate) = S3_GATE_CACHE.get() {
        return Ok(Some(gate));
    }
    let gate = store
        .list_dependency_gates()
        .await?
        .into_iter()
        .find(|gate| gate.dependency_key == LibraryDependency::S3.as_str());
    if let Some(gate) = &gate {
        S3_GATE_CACHE.put(gate.clone());
    }
    Ok(gate)
}

impl LibraryService {
    async fn ensure_active_storage_ready_for(&self, lease_token: Option<Uuid>) -> Result<()> {
        if self.storage.backend() != "s3" {
            return Ok(());
        }
        let gate = cached_s3_gate(&self.store)
            .await?
            .ok_or_else(|| {
                DomainError::unavailable("s3 dependency unavailable: dependency gate is missing")
            })
            .map_err(anyhow::Error::from)?;
        let probe_owned = gate.state == "half_open"
            && lease_token.is_some()
            && gate.probe_lease_token == lease_token;
        if gate.state != "closed" && !probe_owned {
            return Err(DomainError::unavailable(format!(
                "s3 dependency unavailable: state={}{}",
                gate.state,
                gate.last_error
                    .as_deref()
                    .map(|error| format!("; last_error={error}"))
                    .unwrap_or_default()
            ))
            .into());
        }
        Ok(())
    }

    pub(super) async fn write_active_storage(&self, key: &str, bytes: Bytes) -> Result<()> {
        self.write_active_storage_with_lease(key, bytes, None).await
    }

    pub(super) async fn write_active_storage_for_lease(
        &self,
        key: &str,
        bytes: Bytes,
        lease_token: Uuid,
    ) -> Result<()> {
        self.write_active_storage_with_lease(key, bytes, Some(lease_token))
            .await
    }

    async fn write_active_storage_with_lease(
        &self,
        key: &str,
        bytes: Bytes,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.write(key, bytes).await {
            Ok(()) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(())
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(DomainError::unavailable(format!("s3 dependency unavailable: {error}")).into())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn read_active_storage(&self, key: &str) -> Result<Option<Bytes>> {
        self.read_active_storage_with_lease(key, None).await
    }

    pub(super) async fn read_active_storage_for_lease(
        &self,
        key: &str,
        lease_token: Uuid,
    ) -> Result<Option<Bytes>> {
        self.read_active_storage_with_lease(key, Some(lease_token))
            .await
    }

    async fn read_active_storage_with_lease(
        &self,
        key: &str,
        lease_token: Option<Uuid>,
    ) -> Result<Option<Bytes>> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.read(key).await {
            Ok(bytes) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(bytes)
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(DomainError::unavailable(format!("s3 dependency unavailable: {error}")).into())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn exists_active_storage(&self, key: &str) -> Result<bool> {
        self.exists_active_storage_for_lease_context(key, None)
            .await
    }

    pub(super) async fn exists_active_storage_for_lease(
        &self,
        key: &str,
        lease_token: Uuid,
    ) -> Result<bool> {
        self.exists_active_storage_for_lease_context(key, Some(lease_token))
            .await
    }

    async fn exists_active_storage_for_lease_context(
        &self,
        key: &str,
        lease_token: Option<Uuid>,
    ) -> Result<bool> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.exists(key).await {
            Ok(exists) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(exists)
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(DomainError::unavailable(format!("s3 dependency unavailable: {error}")).into())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn delete_active_storage(&self, key: &str) -> Result<()> {
        self.delete_active_storage_with_lease(key, None).await
    }

    pub(super) async fn delete_active_storage_for_lease(
        &self,
        key: &str,
        lease_token: Uuid,
    ) -> Result<()> {
        self.delete_active_storage_with_lease(key, Some(lease_token))
            .await
    }

    async fn delete_active_storage_with_lease(
        &self,
        key: &str,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        self.ensure_active_storage_ready_for(lease_token).await?;
        match self.storage.delete(key).await {
            Ok(()) => {
                if let Some(lease_token) = lease_token {
                    self.note_dependency_success(LibraryDependency::S3, lease_token)
                        .await;
                }
                Ok(())
            }
            Err(error)
                if self.storage.backend() == "s3"
                    && (is_s3_transient_error(&error) || is_configuration_error(&error)) =>
            {
                self.note_dependency_failure_with_lease(
                    LibraryDependency::S3,
                    lease_token.unwrap_or_else(Uuid::nil),
                    &error,
                )
                .await;
                Err(DomainError::unavailable(format!("s3 dependency unavailable: {error}")).into())
            }
            Err(error) => Err(error),
        }
    }
}

/// Typed error for an S3 operation that failed without retry: the message
/// keeps the `s3 operation ... kind=...` shape the dependency classifiers
/// match on, while the variant carries the API status (missing keys are
/// 404, key conflicts are 409, anything else is an upstream failure).
fn s3_operation_error(operation: &str, error: &opendal::Error) -> DomainError {
    let message = format!(
        "s3 operation {operation} failed: kind={:?}: {error}",
        error.kind()
    );
    match error.kind() {
        ErrorKind::NotFound => DomainError::not_found(message),
        ErrorKind::AlreadyExists => DomainError::conflict(message),
        _ => DomainError::upstream_error(message),
    }
}

pub(super) async fn bounded_s3_operation<T, F, Fut>(operation: &str, mut action: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, opendal::Error>>,
{
    let result = timeout(S3_OPERATION_TIMEOUT, async {
        let mut last_error = None;
        let mut retryable_error_seen = false;
        for attempt in 0..=S3_RETRY_LIMIT {
            let result = timeout(S3_ATTEMPT_TIMEOUT, action()).await;
            match result {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(error)) => {
                    if !is_s3_attempt_retryable(&error) {
                        return Err(s3_operation_error(operation, &error).into());
                    }
                    retryable_error_seen = true;
                    last_error = Some(format!("kind={:?}: {error}", error.kind()));
                }
                Err(_) => {
                    retryable_error_seen = true;
                    last_error = Some(format!(
                        "{operation} attempt timed out after {}s",
                        S3_ATTEMPT_TIMEOUT.as_secs()
                    ));
                }
            }
            if attempt < S3_RETRY_LIMIT {
                tokio::time::sleep(Duration::from_millis(200 * (attempt as u64 + 1))).await;
            }
        }

        Err(DomainError::unavailable(format!(
            "s3 operation {operation} failed after {} attempts: {}{}",
            S3_RETRY_LIMIT + 1,
            last_error.unwrap_or_else(|| "unknown error".to_string()),
            if retryable_error_seen {
                "; s3 transient transport failure"
            } else {
                ""
            }
        ))
        .into())
    })
    .await;

    result.unwrap_or_else(|_| {
        Err(DomainError::upstream_timeout(format!(
            "s3 operation {operation} timed out after {}s",
            S3_OPERATION_TIMEOUT.as_secs()
        ))
        .into())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use chrono::Utc;
    use uuid::Uuid;

    use super::{S3_GATE_CACHE, bounded_s3_operation, cached_s3_gate, observe_s3_gate_transition};
    use crate::db::Database;
    use crate::library_store::{DependencyGateRecord, DependencyGateTransition, LibraryStore};
    use crate::services::library::LibraryDependency;

    /// Serializes the tests that mutate the process-wide [`S3_GATE_CACHE`].
    static CACHE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn gate_record() -> DependencyGateRecord {
        DependencyGateRecord {
            dependency_key: LibraryDependency::S3.as_str().to_string(),
            state: "closed".to_string(),
            failure_count: 0,
            next_probe_at: None,
            probe_lease_expires_at: None,
            last_error: None,
            probe_lease_token: Some(Uuid::nil()),
            last_transition_at: Utc::now(),
            last_success_at: None,
        }
    }

    fn transition(dependency: LibraryDependency) -> DependencyGateTransition {
        DependencyGateTransition {
            dependency_key: dependency.canonical_str().to_string(),
            state: "open".to_string(),
            transitioned: true,
        }
    }

    #[tokio::test]
    async fn s3_gate_cache_is_dropped_only_by_s3_state_transitions() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        S3_GATE_CACHE.put(gate_record());

        // A recorded write that did not change state keeps the cached record.
        let unchanged = DependencyGateTransition {
            dependency_key: LibraryDependency::S3.as_str().to_string(),
            state: "closed".to_string(),
            transitioned: false,
        };
        observe_s3_gate_transition(LibraryDependency::S3, Some(&unchanged));
        assert!(S3_GATE_CACHE.get().is_some());

        // Another dependency transitioning must not evict the S3 record.
        observe_s3_gate_transition(
            LibraryDependency::Embedding,
            Some(&transition(LibraryDependency::Embedding)),
        );
        assert!(S3_GATE_CACHE.get().is_some());

        // A real S3 transition evicts it so the next operation re-reads the gate.
        observe_s3_gate_transition(
            LibraryDependency::S3,
            Some(&transition(LibraryDependency::S3)),
        );
        assert!(S3_GATE_CACHE.get().is_none());

        // Leave no cached record behind for the rest of this test binary.
        S3_GATE_CACHE.invalidate();
    }

    #[tokio::test]
    async fn s3_gate_cache_serves_the_last_record_until_this_process_sees_a_transition() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        S3_GATE_CACHE.invalidate();

        let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 gate cache test");
            return;
        };
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let store = LibraryStore::new(db.clone());
        sqlx::query(
            "INSERT INTO context69.library_dependency_gates (dependency_key, state) \
             VALUES ('s3', 'closed') \
             ON CONFLICT (dependency_key) DO UPDATE SET state = 'closed', failure_count = 0, \
             last_error = NULL, next_probe_at = NULL, probe_lease_token = NULL, \
             probe_lease_expires_at = NULL, updated_at = now()",
        )
        .execute(db.pool())
        .await
        .expect("seed closed s3 gate");

        let cached = cached_s3_gate(&store)
            .await
            .expect("read s3 gate")
            .expect("s3 gate row");
        assert_eq!(cached.state, "closed");

        // Another instance trips the gate. Inside the TTL this process keeps
        // serving the cached record instead of rescanning the gate table.
        store
            .record_dependency_failure(
                LibraryDependency::S3.as_str(),
                Uuid::nil(),
                "cache test failure",
            )
            .await
            .expect("trip s3 gate");
        let served = cached_s3_gate(&store)
            .await
            .expect("read s3 gate")
            .expect("s3 gate row");
        assert_eq!(
            served.state, "closed",
            "the cached gate record is served inside the TTL"
        );

        // Once this process records the transition, the next read is fresh.
        observe_s3_gate_transition(
            LibraryDependency::S3,
            Some(&transition(LibraryDependency::S3)),
        );
        let refreshed = cached_s3_gate(&store)
            .await
            .expect("read s3 gate")
            .expect("s3 gate row");
        assert_eq!(
            refreshed.state, "open",
            "an observed transition drops the cached record"
        );

        sqlx::query(
            "UPDATE context69.library_dependency_gates SET state = 'closed', failure_count = 0, \
             last_error = NULL, next_probe_at = NULL, probe_lease_token = NULL, \
             probe_lease_expires_at = NULL, updated_at = now() WHERE dependency_key = 's3'",
        )
        .execute(db.pool())
        .await
        .expect("restore s3 gate");
        S3_GATE_CACHE.invalidate();
    }

    #[tokio::test]
    async fn does_not_retry_permanent_s3_errors() {
        let calls = Arc::new(AtomicUsize::new(0));
        let result = bounded_s3_operation("write", {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                async {
                    Err::<(), _>(opendal::Error::new(
                        opendal::ErrorKind::AlreadyExists,
                        "object already exists",
                    ))
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn retries_temporary_s3_errors_within_the_attempt_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let result = bounded_s3_operation("write", {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                async {
                    Err::<(), _>(
                        opendal::Error::new(opendal::ErrorKind::Unexpected, "upstream timeout")
                            .set_temporary(),
                    )
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }
}
