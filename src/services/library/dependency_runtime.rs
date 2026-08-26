use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{
    LibraryDependencyGateResponse, LibraryProcessingMetric, LibraryProcessingQueueHealth,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

pub(super) use super::dependency_errors::{
    dependency_is_transient, is_configuration_error, is_s3_error, redact_dependency_error,
};
pub(super) use super::dependency_storage::bounded_s3_operation;
use super::{LibraryDependency, LibraryService};
use crate::library_store::LibraryStore;

const PROBE_LEASE_TTL_SECS: i64 = super::LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS;

impl LibraryService {
    pub(crate) async fn initialize_dependency_gates(&self) -> Result<()> {
        self.refresh_dependency_configuration().await
    }

    pub(crate) async fn dependency_wait_until(
        &self,
        dependency_key: &str,
        lease_token: Uuid,
    ) -> Result<Option<DateTime<Utc>>> {
        let canonical = LibraryDependency::canonical_key(dependency_key);
        if canonical == LibraryDependency::S3.canonical_str() && self.storage.backend() != "s3" {
            return Ok(None);
        }
        if (canonical == LibraryDependency::Embedding.canonical_str()
            || canonical == LibraryDependency::Qdrant.canonical_str())
            && self.runtime.is_none()
        {
            return Ok(Some(Utc::now() + chrono::Duration::seconds(30)));
        }
        let gates = self.store.list_dependency_gates().await?;
        let gate = find_gate_by_canonical(&gates, canonical);
        let Some(gate) = gate else {
            return Ok(Some(Utc::now() + chrono::Duration::seconds(30)));
        };
        if gate.state == "closed" || gate.probe_lease_token == Some(lease_token) {
            return Ok(None);
        }

        let now = Utc::now();
        let probe_due = gate
            .next_probe_at
            .map(|next_probe_at| next_probe_at <= now)
            .unwrap_or(gate.state == "half_open");
        if probe_due
            && let Some(transition) = self
                .store
                .reserve_dependency_probe(&gate.dependency_key, lease_token, PROBE_LEASE_TTL_SECS)
                .await?
        {
            log_dependency_transition(&transition);
            return Ok(None);
        }

        Ok(Some(
            gate.probe_lease_expires_at
                .or(gate.next_probe_at)
                .filter(|value| *value > now)
                .unwrap_or_else(|| now + chrono::Duration::seconds(30)),
        ))
    }

    pub(crate) async fn refresh_dependency_configuration(&self) -> Result<()> {
        if let Some(configuration_fingerprint) = &self.s3_configuration_fingerprint {
            self.configure_dependency_gate(
                LibraryDependency::S3,
                true,
                None,
                configuration_fingerprint,
            )
            .await?;
        }
        // Ensure canonical gates exist before configuring. The legacy
        // embedding_vector row already exists via the original migration; the new
        // canonical embedding and qdrant rows are created idempotently here.
        self.store
            .ensure_dependency_gate(LibraryDependency::Embedding.canonical_str())
            .await?;
        self.store
            .ensure_dependency_gate(LibraryDependency::Qdrant.canonical_str())
            .await?;
        // Keep the legacy alias row present for old DBs that still have it.
        self.store
            .ensure_dependency_gate(LibraryDependency::EmbeddingVector.as_str())
            .await?;
        let configured = self.embedding_vector_configured;
        let fingerprint = self.embedding_vector_configuration_fingerprint.clone();
        let not_configured_error =
            (!configured).then_some("configuration: embedding/vector runtime is not configured");
        for dependency in [
            LibraryDependency::Embedding,
            LibraryDependency::Qdrant,
            LibraryDependency::EmbeddingVector,
        ] {
            self.configure_dependency_gate(
                dependency,
                configured,
                not_configured_error,
                &fingerprint,
            )
            .await?;
        }
        if configured && self.runtime.is_none() {
            let error = anyhow!("embedding/vector runtime is unavailable");
            // Trip both canonical gates independently; the legacy alias is
            // also tripped so old rows do not stay closed while the new gates
            // are open.
            for dependency in [
                LibraryDependency::Embedding,
                LibraryDependency::Qdrant,
                LibraryDependency::EmbeddingVector,
            ] {
                self.note_dependency_failure(dependency, &error).await;
            }
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
        match dependency.canonical() {
            LibraryDependency::S3 => self
                .s3_configuration_fingerprint
                .clone()
                .unwrap_or_else(|| configuration_fingerprint(&["s3", "disabled"])),
            LibraryDependency::Embedding
            | LibraryDependency::Qdrant
            | LibraryDependency::EmbeddingVector => {
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
        // Persist using the canonical key so new deployments write `embedding`
        // and `qdrant`. The legacy `embedding_vector` row is still written
        // when the caller explicitly passes that variant so existing rows stay
        // in sync.
        let key = if dependency == LibraryDependency::EmbeddingVector {
            dependency.as_str()
        } else {
            dependency.canonical_str()
        };
        if let Some(transition) = self
            .store
            .configure_dependency_gate(key, configured, error, configuration_fingerprint)
            .await?
        {
            log_dependency_transition(&transition);
        }
        Ok(())
    }

    pub(crate) async fn processing_health(
        &self,
    ) -> Result<(
        bool,
        Vec<LibraryDependencyGateResponse>,
        LibraryProcessingQueueHealth,
    )> {
        let mut gates = self.store.list_dependency_gates().await?;
        let queue = self.db.task_processing_health().await?;
        let mut required_dependencies = vec![
            LibraryDependency::Embedding.canonical_str(),
            LibraryDependency::Qdrant.canonical_str(),
        ];
        if queue.docling_required_count > 0 {
            required_dependencies.push(LibraryDependency::Docling.canonical_str());
        }
        if self.storage.backend() == "s3" {
            required_dependencies.push(LibraryDependency::S3.canonical_str());
        }
        let ready = required_dependencies.iter().all(|dependency| {
            gates.iter().any(|gate| {
                LibraryDependency::canonical_key(&gate.dependency_key) == *dependency
                    && gate.state == "closed"
            })
        });
        // Backward compatibility: if the canonical `embedding` gate exists but
        // the legacy `embedding_vector` row is absent (fresh DB after the
        // split), synthesize it so old clients that still query for
        // `embedding_vector` see a consistent state. If the legacy row already
        // exists we keep the stored state (which is kept in sync via
        // refresh_dependency_configuration).
        let has_legacy = gates
            .iter()
            .any(|gate| gate.dependency_key == LibraryDependency::EmbeddingVector.as_str());
        if !has_legacy
            && let Some(canonical) = gates
                .iter()
                .find(|gate| {
                    LibraryDependency::canonical_key(&gate.dependency_key)
                        == LibraryDependency::Embedding.canonical_str()
                })
                .cloned()
        {
            gates.push(crate::library_store::DependencyGateRecord {
                dependency_key: LibraryDependency::EmbeddingVector.as_str().to_string(),
                state: canonical.state.clone(),
                failure_count: canonical.failure_count,
                next_probe_at: canonical.next_probe_at,
                probe_lease_expires_at: canonical.probe_lease_expires_at,
                last_error: canonical.last_error.clone(),
                probe_lease_token: canonical.probe_lease_token,
                last_transition_at: canonical.last_transition_at,
                last_success_at: canonical.last_success_at,
            });
        }
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
        let status_counts = parse_processing_metrics(queue.status_counts)?;
        let stage_counts = parse_processing_metrics(queue.stage_counts)?;
        let waiting_reason_counts = parse_processing_metrics(queue.waiting_reason_counts)?;
        let dependency_counts = parse_processing_metrics(queue.dependency_counts)?;
        let processed_last_hour = non_negative_count(queue.processed_last_hour)?;
        let failed_last_hour = non_negative_count(queue.failed_last_hour)?;
        let processing_rate_per_minute = processed_last_hour as f64 / 60.0;
        let failure_rate_percent = if processed_last_hour == 0 {
            0.0
        } else {
            failed_last_hour as f64 * 100.0 / processed_last_hour as f64
        };
        Ok((
            ready,
            response,
            LibraryProcessingQueueHealth {
                pending_count: non_negative_count(queue.pending_count)?,
                queued_count: non_negative_count(queue.queued_count)?,
                oldest_pending_age_seconds: queue_age_seconds(queue.oldest_pending_at, now),
                oldest_queued_age_seconds: queue_age_seconds(queue.oldest_queued_at, now),
                oldest_waiting_age_seconds: queue_age_seconds(queue.oldest_waiting_at, now),
                recent_failure_count: non_negative_count(queue.recent_failure_count)?,
                docling_dependency_waiting_count: non_negative_count(
                    queue.docling_dependency_waiting_count,
                )?,
                stale_waiting_count: non_negative_count(queue.stale_waiting_count)?,
                expired_active_external_jobs: non_negative_count(queue.expired_active_jobs)?,
                active_external_jobs: non_negative_count(queue.active_jobs)?,
                status_counts,
                stage_counts,
                waiting_reason_counts,
                dependency_counts,
                processed_last_hour,
                failed_last_hour,
                processing_rate_per_minute,
                failure_rate_percent,
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
        let canonical = dependency.canonical();
        let error_message = redact_dependency_error(error);
        let configuration_fingerprint = self.dependency_configuration_fingerprint(canonical).await;
        let result = if is_configuration_error(error) {
            self.store
                .configure_dependency_gate(
                    canonical.canonical_str(),
                    false,
                    Some(&format!("configuration: {error_message}")),
                    &configuration_fingerprint,
                )
                .await
        } else if dependency_is_transient(canonical, error) {
            self.store
                .record_dependency_failure(canonical.canonical_str(), lease_token, &error_message)
                .await
        } else {
            return;
        };
        match result {
            Ok(Some(transition)) => log_dependency_transition(&transition),
            Ok(None) => {}
            Err(record_error) => {
                warn!(
                    dependency = canonical.canonical_str(),
                    error = %record_error,
                    "failed to persist library dependency gate failure"
                );
            }
        }
        // Keep the legacy alias in sync with the canonical embedding gate so
        // old rows/tasks do not get stranded and health checks remain coherent
        // for clients still reading `embedding_vector`.
        if canonical == LibraryDependency::Embedding {
            let legacy_result = if is_configuration_error(error) {
                self.store
                    .configure_dependency_gate(
                        LibraryDependency::EmbeddingVector.as_str(),
                        false,
                        Some(&format!("configuration: {error_message}")),
                        &configuration_fingerprint,
                    )
                    .await
            } else if dependency_is_transient(canonical, error) {
                self.store
                    .record_dependency_failure(
                        LibraryDependency::EmbeddingVector.as_str(),
                        lease_token,
                        &error_message,
                    )
                    .await
            } else {
                return;
            };
            if let Ok(Some(transition)) = legacy_result {
                log_dependency_transition(&transition);
            }
        }
    }

    pub(super) async fn note_dependency_success(
        &self,
        dependency: LibraryDependency,
        lease_token: Uuid,
    ) {
        let canonical = dependency.canonical();
        let key = canonical.canonical_str();
        match self.store.record_dependency_success(key, lease_token).await {
            Ok(Some(transition)) => log_dependency_transition(&transition),
            Ok(None) => {}
            Err(error) => {
                warn!(
                    dependency = key,
                    %error,
                    "failed to persist library dependency gate recovery"
                );
            }
        }
        // Mirror success to the legacy alias when the canonical is embedding.
        if canonical == LibraryDependency::Embedding
            && let Ok(Some(transition)) = self
                .store
                .record_dependency_success(LibraryDependency::EmbeddingVector.as_str(), lease_token)
                .await
        {
            log_dependency_transition(&transition);
        }
    }
}

pub(crate) async fn report_dependency_processing_error_with_lease(
    store: &LibraryStore,
    configuration_fingerprint: &str,
    dependency: LibraryDependency,
    lease_token: Uuid,
    error: &str,
) -> Result<bool> {
    let error = anyhow::anyhow!(error.to_string());
    let error_message = redact_dependency_error(&error);
    let canonical = dependency.canonical();
    let result = if is_configuration_error(&error) {
        store
            .configure_dependency_gate(
                canonical.canonical_str(),
                false,
                Some(&format!("configuration: {error_message}")),
                configuration_fingerprint,
            )
            .await?
    } else if dependency_is_transient(canonical, &error) {
        store
            .record_dependency_failure(canonical.canonical_str(), lease_token, &error_message)
            .await?
    } else {
        return Ok(false);
    };
    if let Some(transition) = result {
        log_dependency_transition(&transition);
    }
    // Mirror embedding results to the legacy alias.
    if canonical == LibraryDependency::Embedding {
        let legacy_result = if is_configuration_error(&error) {
            store
                .configure_dependency_gate(
                    LibraryDependency::EmbeddingVector.as_str(),
                    false,
                    Some(&format!("configuration: {error_message}")),
                    configuration_fingerprint,
                )
                .await?
        } else if dependency_is_transient(canonical, &error) {
            store
                .record_dependency_failure(
                    LibraryDependency::EmbeddingVector.as_str(),
                    lease_token,
                    &error_message,
                )
                .await?
        } else {
            None
        };
        if let Some(transition) = legacy_result {
            log_dependency_transition(&transition);
        }
    }
    Ok(true)
}

pub(crate) async fn report_embedding_vector_processing_error_with_lease(
    store: &LibraryStore,
    configuration_fingerprint: &str,
    lease_token: Uuid,
    error: &str,
) -> Result<bool> {
    // Preserve the old symbol for callers that still import it; route
    // automatically based on the error content so qdrant failures do not stay
    // hidden behind the embedding gate.
    let is_qdrant = error.to_ascii_lowercase().contains("qdrant");
    let dependency = if is_qdrant {
        LibraryDependency::Qdrant
    } else {
        LibraryDependency::Embedding
    };
    report_dependency_processing_error_with_lease(
        store,
        configuration_fingerprint,
        dependency,
        lease_token,
        error,
    )
    .await
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

fn find_gate_by_canonical<'a>(
    gates: &'a [crate::library_store::DependencyGateRecord],
    canonical: &str,
) -> Option<&'a crate::library_store::DependencyGateRecord> {
    // Prefer an exact canonical row; fall back to the legacy alias that
    // canonicalizes to the same key (e.g. `embedding_vector` -> `embedding`).
    gates
        .iter()
        .find(|gate| gate.dependency_key == canonical)
        .or_else(|| {
            gates
                .iter()
                .find(|gate| LibraryDependency::canonical_key(&gate.dependency_key) == canonical)
        })
}

fn non_negative_count(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| anyhow!("negative library processing health count"))
}

fn queue_age_seconds(timestamp: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<u64> {
    timestamp.map(|timestamp| now.signed_duration_since(timestamp).num_seconds().max(0) as u64)
}

fn parse_processing_metrics(value: Value) -> Result<Vec<LibraryProcessingMetric>> {
    serde_json::from_value(value).map_err(Into::into)
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
    append_optional(&mut parts, config.vlm.picture_description_preset.as_deref());
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

/// Pure, dependency-free classifier that decides whether a Docling poll
/// error is transient (and therefore should open the dependency gate) or
/// terminal (and therefore should fail the item). Structured
/// `PdfConvertError` status codes win over string matching so HTTP 404, 401,
/// and 403 are never treated as transient.
pub(super) fn is_docling_transient(
    docling_error: Option<&docling_convert::PdfConvertError>,
    context_error: &anyhow::Error,
) -> bool {
    if let Some(error) = docling_error {
        match docling_error_status_code(error) {
            Some(404 | 401 | 403) => return false,
            Some(status) if (500..600).contains(&status) => return true,
            Some(429) => return true,
            Some(_) => return false,
            None => {}
        }
    }
    dependency_is_transient(LibraryDependency::Docling, context_error)
}

/// The released Docling client exposes the HTTP status in its error display,
/// but older releases do not expose a status accessor. Keep the compatibility
/// parsing in one place so status handling does not spread through the task
/// state machine.
pub(super) fn docling_error_status_code(error: &docling_convert::PdfConvertError) -> Option<u16> {
    let display = error.to_string();
    let (_, value) = display.split_once("HTTP ")?;
    let digits: String = value
        .chars()
        .take_while(char::is_ascii_digit)
        .take(3)
        .collect();
    (digits.len() == 3).then(|| digits.parse().ok()).flatten()
}

pub(super) fn is_docling_remote_task_not_found(error: &docling_convert::PdfConvertError) -> bool {
    docling_error_status_code(error) == Some(404)
}

/// Test-only free-function wrapper, kept as a separate name so callers in
/// sibling modules can import it without dragging in the `LibraryService`
/// lifetime. Mirrors `is_docling_transient`.
#[cfg(test)]
pub(crate) fn is_docling_transient_error_for_test(
    docling_error: Option<&docling_convert::PdfConvertError>,
    context_error: &anyhow::Error,
) -> bool {
    is_docling_transient(docling_error, context_error)
}
