use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct DependencyGateRecord {
    pub dependency_key: String,
    pub state: String,
    pub failure_count: i32,
    pub next_probe_at: Option<DateTime<Utc>>,
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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct IngestClaim {
    pub job_id: Uuid,
    pub file_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub requires_docling: bool,
    pub lease_token: Uuid,
    pub storage_backend: String,
    pub section_payload: Option<Value>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PendingIngestDependencies {
    pub has_pending: bool,
    pub requires_docling: bool,
    pub requires_s3: bool,
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

    pub(crate) async fn renew_dependency_probe(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/dependency_gates/renew_probe.sql",
            dependency_key,
            lease_token,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?
        .is_some())
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

    pub(crate) async fn claim_next_ingest_job(
        &self,
        lease_token: Uuid,
        lease_ttl_secs: i64,
        storage_backend: &str,
    ) -> Result<Option<IngestClaim>> {
        Ok(sqlx::query_file_as!(
            IngestClaim,
            "src/sql/library_store/jobs/claim_next.sql",
            lease_token,
            lease_ttl_secs,
            storage_backend
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn pending_ingest_dependencies(
        &self,
        storage_backend: &str,
    ) -> Result<PendingIngestDependencies> {
        Ok(sqlx::query_file_as!(
            PendingIngestDependencies,
            "src/sql/library_store/jobs/pending_dependencies.sql",
            storage_backend
        )
        .fetch_one(self.db.pool())
        .await?)
    }

    pub(crate) async fn release_ingest_job(&self, job_id: Uuid, lease_token: Uuid) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/jobs/release.sql",
            job_id,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?
        .is_some())
    }

    pub(crate) async fn recover_expired_ingest_jobs(&self) -> Result<()> {
        sqlx::query_file!("src/sql/library_store/jobs/recover_expired.sql")
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub(crate) async fn heartbeat_ingest_job(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/jobs/heartbeat.sql",
            job_id,
            lease_token,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?
        .is_some())
    }
}
