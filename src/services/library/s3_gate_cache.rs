//! Process-wide cache for the S3 dependency gate record.

use std::time::Duration;

use anyhow::Result;

use super::LibraryDependency;
use super::ttl_cache::TtlCache;
use crate::library_store::{DependencyGateRecord, DependencyGateTransition, LibraryStore};

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
pub(super) async fn cached_s3_gate(store: &LibraryStore) -> Result<Option<DependencyGateRecord>> {
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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::{S3_GATE_CACHE, cached_s3_gate, observe_s3_gate_transition};
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
}
