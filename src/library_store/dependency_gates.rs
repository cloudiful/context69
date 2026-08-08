use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct DependencyGateRecord {
    pub dependency_key: String,
    pub state: String,
    pub failure_count: i32,
    pub next_probe_at: Option<DateTime<Utc>>,
    pub probe_lease_expires_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub probe_lease_token: Option<Uuid>,
    pub last_transition_at: DateTime<Utc>,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct DependencyGateTransition {
    pub dependency_key: String,
    pub state: String,
    pub transitioned: bool,
}

impl LibraryStore {
    pub(crate) async fn list_dependency_gates(&self) -> Result<Vec<DependencyGateRecord>> {
        Ok(sqlx::query_file_as!(
            DependencyGateRecord,
            "src/sql/library_store/dependency_gates/get.sql"
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    pub(crate) async fn configure_dependency_gate(
        &self,
        dependency_key: &str,
        configured: bool,
        error: Option<&str>,
        configuration_fingerprint: &str,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/configure.sql",
            dependency_key,
            configured,
            error,
            configuration_fingerprint
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn reserve_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/reserve_probe.sql",
            dependency_key,
            lease_token,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn record_dependency_success(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/success.sql",
            dependency_key,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn record_dependency_failure(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
        error: &str,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/failure.sql",
            dependency_key,
            lease_token,
            error
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn abandon_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/abandon_probe.sql",
            dependency_key,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn release_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DependencyGateTransition>> {
        Ok(sqlx::query_file_as!(
            DependencyGateTransition,
            "src/sql/library_store/dependency_gates/release_probe.sql",
            dependency_key,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?)
    }
}
