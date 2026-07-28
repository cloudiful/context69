use std::time::Duration;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{LibraryDependencyGateResponse, LibraryProcessingQueueHealth};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

pub(super) use super::dependency_errors::{
    dependency_is_transient, is_configuration_error, is_s3_error, is_s3_transient_error,
    is_transient_download_error, redact_dependency_error,
};
pub(super) use super::dependency_storage::bounded_s3_operation;
use super::{LibraryDependency, LibraryService};
use crate::library_store::LibraryStore;

const INGEST_LEASE_TTL_SECS: i64 = 120;
const INGEST_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const INGEST_POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROBE_LEASE_TTL_SECS: i64 = super::LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS;

impl LibraryService {
    pub(crate) async fn resume_ingest_jobs(&self) -> Result<()> {
        self.refresh_dependency_configuration().await?;
        self.store.recover_expired_ingest_jobs().await?;
        self.start_ingest_workers();
        self.notify_ingest_worker();
        Ok(())
    }

    pub(super) async fn refresh_dependency_configuration(&self) -> Result<()> {
        if let Some(configuration_fingerprint) = &self.s3_configuration_fingerprint {
            self.configure_dependency_gate(
                LibraryDependency::S3,
                true,
                None,
                configuration_fingerprint,
            )
            .await?;
        }
        self.configure_dependency_gate(
            LibraryDependency::EmbeddingVector,
            self.embedding_vector_configured,
            (!self.embedding_vector_configured)
                .then_some("configuration: embedding/vector runtime is not configured"),
            &self.embedding_vector_configuration_fingerprint,
        )
        .await?;
        if self.embedding_vector_configured && self.runtime.is_none() {
            let error = anyhow!("embedding/vector runtime is unavailable");
            self.note_dependency_failure(LibraryDependency::EmbeddingVector, &error)
                .await;
        }

        match self.settings.resolve_docling_config().await {
            Ok(Some(config)) => {
                let configuration_fingerprint = docling_configuration_fingerprint(Some(&config));
                self.configure_dependency_gate(
                    LibraryDependency::Docling,
                    true,
                    None,
                    &configuration_fingerprint,
                )
                .await?;
            }
            Ok(None) => {
                self.configure_dependency_gate(
                    LibraryDependency::Docling,
                    false,
                    Some("configuration: docling runtime is not configured"),
                    &docling_configuration_fingerprint(None),
                )
                .await?;
            }
            Err(error) => {
                self.configure_dependency_gate(
                    LibraryDependency::Docling,
                    false,
                    Some(&format!("configuration: {error}")),
                    &configuration_fingerprint(&["docling", "resolution-error"]),
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn dependency_configuration_fingerprint(&self, dependency: LibraryDependency) -> String {
        match dependency {
            LibraryDependency::S3 => self
                .s3_configuration_fingerprint
                .clone()
                .unwrap_or_else(|| configuration_fingerprint(&["s3", "disabled"])),
            LibraryDependency::EmbeddingVector => {
                self.embedding_vector_configuration_fingerprint.clone()
            }
            LibraryDependency::Docling => match self.settings.resolve_docling_config().await {
                Ok(Some(config)) => docling_configuration_fingerprint(Some(&config)),
                Ok(None) => docling_configuration_fingerprint(None),
                Err(_) => configuration_fingerprint(&["docling", "resolution-error"]),
            },
        }
    }

    async fn configure_dependency_gate(
        &self,
        dependency: LibraryDependency,
        configured: bool,
        error: Option<&str>,
        configuration_fingerprint: &str,
    ) -> Result<()> {
        if let Some(transition) = self
            .store
            .configure_dependency_gate(
                dependency.as_str(),
                configured,
                error,
                configuration_fingerprint,
            )
            .await?
        {
            log_dependency_transition(&transition);
        }
        Ok(())
    }

    pub(super) fn start_ingest_workers(&self) {
        if self
            .ingest_workers_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }

        for worker_id in 0..self.ingest_worker_count {
            let service = self.clone();
            tokio::spawn(async move {
                service.run_ingest_worker(worker_id).await;
            });
        }
    }

    pub(super) fn notify_ingest_worker(&self) {
        self.ingest_wakeup.notify_waiters();
    }

    pub(crate) async fn processing_health(
        &self,
    ) -> Result<(
        bool,
        Vec<LibraryDependencyGateResponse>,
        LibraryProcessingQueueHealth,
    )> {
        let gates = self.store.list_dependency_gates().await?;
        let queue = self.store.processing_queue_health().await?;
        let mut required_dependencies = vec![LibraryDependency::EmbeddingVector.as_str()];
        if queue.docling_required_count > 0 {
            required_dependencies.push(LibraryDependency::Docling.as_str());
        }
        if self.storage.backend() == "s3" {
            required_dependencies.push(LibraryDependency::S3.as_str());
        }
        let ready = required_dependencies.iter().all(|dependency| {
            gates
                .iter()
                .any(|gate| gate.dependency_key == *dependency && gate.state == "closed")
        });
        let response = gates
            .into_iter()
            .map(|gate| LibraryDependencyGateResponse {
                dependency_key: gate.dependency_key,
                state: gate.state,
                failure_count: u32::try_from(gate.failure_count.max(0)).unwrap_or(u32::MAX),
                next_probe_at: gate.next_probe_at,
                last_error: gate.last_error,
                last_transition_at: gate.last_transition_at,
                last_success_at: gate.last_success_at,
            })
            .collect();
        let now = Utc::now();
        Ok((
            ready,
            response,
            LibraryProcessingQueueHealth {
                pending_count: non_negative_count(queue.pending_count)?,
                queued_count: non_negative_count(queue.queued_count)?,
                oldest_pending_age_seconds: queue_age_seconds(queue.oldest_pending_at, now),
                oldest_queued_age_seconds: queue_age_seconds(queue.oldest_queued_at, now),
                recent_failure_count: non_negative_count(queue.recent_failure_count)?,
            },
        ))
    }

    pub(super) async fn note_dependency_failure(
        &self,
        dependency: LibraryDependency,
        error: &anyhow::Error,
    ) {
        self.note_dependency_failure_with_lease(dependency, Uuid::nil(), error)
            .await;
    }

    pub(super) async fn note_dependency_failure_with_lease(
        &self,
        dependency: LibraryDependency,
        lease_token: Uuid,
        error: &anyhow::Error,
    ) {
        let error_message = redact_dependency_error(error);
        let configuration_fingerprint = self.dependency_configuration_fingerprint(dependency).await;
        let result = if is_configuration_error(error) {
            self.store
                .configure_dependency_gate(
                    dependency.as_str(),
                    false,
                    Some(&format!("configuration: {error_message}")),
                    &configuration_fingerprint,
                )
                .await
        } else if dependency_is_transient(dependency, error) {
            self.store
                .record_dependency_failure(dependency.as_str(), lease_token, &error_message)
                .await
        } else {
            return;
        };
        match result {
            Ok(Some(transition)) => log_dependency_transition(&transition),
            Ok(None) => {}
            Err(record_error) => {
                warn!(
                    dependency = dependency.as_str(),
                    error = %record_error,
                    "failed to persist library dependency gate failure"
                );
            }
        }
    }

    pub(super) async fn note_dependency_success(
        &self,
        dependency: LibraryDependency,
        lease_token: Uuid,
    ) {
        match self
            .store
            .record_dependency_success(dependency.as_str(), lease_token)
            .await
        {
            Ok(Some(transition)) => log_dependency_transition(&transition),
            Ok(None) => {}
            Err(error) => {
                warn!(
                    dependency = dependency.as_str(),
                    %error,
                    "failed to persist library dependency gate recovery"
                );
            }
        }
    }

    pub(super) async fn wait_for_ingest_work(&self) {
        tokio::select! {
            _ = self.ingest_wakeup.notified() => {}
            _ = tokio::time::sleep(INGEST_POLL_INTERVAL) => {}
        }
    }
}

pub(crate) async fn report_embedding_vector_processing_error_with_lease(
    store: &LibraryStore,
    configuration_fingerprint: &str,
    lease_token: Uuid,
    error: &str,
) -> Result<bool> {
    let error = anyhow::anyhow!(error.to_string());
    let error_message = redact_dependency_error(&error);
    let result = if is_configuration_error(&error) {
        store
            .configure_dependency_gate(
                LibraryDependency::EmbeddingVector.as_str(),
                false,
                Some(&format!("configuration: {error_message}")),
                configuration_fingerprint,
            )
            .await?
    } else if dependency_is_transient(LibraryDependency::EmbeddingVector, &error) {
        store
            .record_dependency_failure(
                LibraryDependency::EmbeddingVector.as_str(),
                lease_token,
                &error_message,
            )
            .await?
    } else {
        return Ok(false);
    };
    if let Some(transition) = result {
        log_dependency_transition(&transition);
    }
    Ok(true)
}

pub(crate) fn log_dependency_transition(
    transition: &crate::library_store::DependencyGateTransition,
) {
    if transition.transitioned {
        info!(
            dependency = %transition.dependency_key,
            state = %transition.state,
            "library dependency gate transitioned"
        );
    }
}

fn non_negative_count(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| anyhow!("negative library processing health count"))
}

fn queue_age_seconds(timestamp: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<u64> {
    timestamp.map(|timestamp| now.signed_duration_since(timestamp).num_seconds().max(0) as u64)
}

pub(super) const fn ingest_lease_ttl_secs() -> i64 {
    INGEST_LEASE_TTL_SECS
}

pub(super) const fn ingest_heartbeat_interval() -> Duration {
    INGEST_HEARTBEAT_INTERVAL
}

pub(super) const fn probe_lease_ttl_secs() -> i64 {
    PROBE_LEASE_TTL_SECS
}

pub(super) fn s3_configuration_fingerprint(config: &crate::config::S3StorageConfig) -> String {
    configuration_fingerprint(&[
        "s3".to_string(),
        config.endpoint.trim_end_matches('/').to_string(),
        config.region.trim().to_string(),
        config.bucket.trim().to_string(),
        config.prefix.trim_matches('/').to_string(),
        config.path_style.to_string(),
        config.access_key.clone(),
        config.secret_key.clone(),
    ])
}

fn docling_configuration_fingerprint(config: Option<&crate::docling::DoclingConfig>) -> String {
    let mut parts = vec!["docling".to_string()];
    let Some(config) = config else {
        parts.push("disabled".to_string());
        return configuration_fingerprint(&parts);
    };

    parts.extend([
        config.connection.base_url.trim_end_matches('/').to_string(),
        config.connection.timeout.as_secs().to_string(),
        config.connection.poll_interval.as_secs().to_string(),
        config.connection.task_timeout.as_secs().to_string(),
    ]);
    append_optional(&mut parts, config.vlm.openai_base_url.as_deref());
    append_optional(&mut parts, config.vlm.api_key.as_deref());
    append_optional(&mut parts, config.vlm.vlm_pipeline_model.as_deref());
    append_optional(&mut parts, config.vlm.picture_description_model.as_deref());
    append_optional(&mut parts, config.vlm.code_formula_model.as_deref());
    configuration_fingerprint(&parts)
}

fn append_optional(parts: &mut Vec<String>, value: Option<&str>) {
    parts.push(value.unwrap_or("<none>").trim().to_string());
}

fn configuration_fingerprint(parts: &[impl AsRef<str>]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        let part = part.as_ref();
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
