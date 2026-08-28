use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Utc;
use context69_contracts::ExtractionFailureClass;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use super::ExtractionService;
use crate::{
    ExtractionPublication,
    providers::{
        ProviderExtractionRequest, classify_error, extract, failure_class_as_str, next_retry_delay,
    },
    store::{ExtractionAttempt, ExtractionVersionInput, FinishExtractionJob},
};

const READINESS_POLL_INTERVAL: Duration = Duration::from_secs(5);
const MAX_ATTEMPTS: i32 = 3;

impl ExtractionService {
    pub(super) async fn run_pending(&self) -> Result<()> {
        loop {
            let guard = match self.worker_lock.try_lock() {
                Ok(g) => g,
                Err(_) => return Ok(()),
            };
            let ids = self.store.pending_ids().await?;
            if ids.is_empty() {
                let next_retry = self.store.next_pending_at().await?;
                if let Some(next_at) = next_retry {
                    // Release lock before sleeping so a newly enqueued due job can wake via spawn_worker.
                    drop(guard);
                    let now = Utc::now();
                    let sleep_duration = if next_at > now {
                        let diff = (next_at - now).to_std().unwrap_or(Duration::from_secs(0));
                        diff.min(Duration::from_secs(5))
                    } else {
                        Duration::from_millis(200)
                    };
                    if sleep_duration > Duration::from_millis(0) {
                        tokio::time::sleep(sleep_duration).await;
                    }
                    continue;
                }
                return Ok(());
            }
            if !self.readiness.is_ready().await? {
                drop(guard);
                tokio::time::sleep(READINESS_POLL_INTERVAL).await;
                continue;
            }
            let mut tasks = Vec::with_capacity(ids.len());
            for id in ids {
                let service = self.clone();
                tasks.push(tokio::spawn(async move {
                    let _permit = service.semaphore.acquire().await?;
                    service.run_job(id).await
                }));
            }
            // Keep guard held while tasks run to prevent concurrent run_pending loops;
            // tasks themselves run concurrently but are bounded by the semaphore.
            for task in tasks {
                if let Err(error) = task.await? {
                    warn!(%error, "extraction job failed");
                }
            }
            drop(guard);
        }
    }

    async fn run_job(&self, id: Uuid) -> Result<bool> {
        let Some(job) = self.store.claim_job(id).await? else {
            return Ok(false);
        };
        if !self.readiness.is_ready().await? {
            self.store.release_claimed_job(id).await?;
            return Ok(false);
        }
        let document = self.store.document(job.document_id).await?;
        if document.record_hash != job.source_record_hash {
            self.store
                .finish_job(FinishExtractionJob {
                    id,
                    status: "failed",
                    provider_key: None,
                    provider_config_hash: None,
                    error_message: Some("source document changed"),
                    failure_class: Some(failure_class_as_str(ExtractionFailureClass::Permanent)),
                    next_attempt_at: None,
                })
                .await?;
            warn!(
                job_id = %id,
                template_key = %job.template_key,
                attempt = job.attempt_count,
                failure_class = "permanent",
                latency_ms = 0,
                "extraction failed: source document changed"
            );
            return Ok(false);
        }
        let Some(template) = self.store.template(&job.template_key).await? else {
            self.store
                .finish_job(FinishExtractionJob {
                    id,
                    status: "failed",
                    provider_key: None,
                    provider_config_hash: None,
                    error_message: Some("extraction template missing"),
                    failure_class: Some(failure_class_as_str(ExtractionFailureClass::Permanent)),
                    next_attempt_at: None,
                })
                .await?;
            warn!(
                job_id = %id,
                template_key = %job.template_key,
                attempt = job.attempt_count,
                failure_class = "permanent",
                latency_ms = 0,
                "extraction failed: template missing"
            );
            return Ok(false);
        };
        if !template.enabled {
            self.store
                .finish_job(FinishExtractionJob {
                    id,
                    status: "skipped",
                    provider_key: None,
                    provider_config_hash: None,
                    error_message: None,
                    failure_class: None,
                    next_attempt_at: None,
                })
                .await?;
            return Ok(false);
        }
        let Some(provider) = self.store.provider().await? else {
            self.store
                .finish_job(FinishExtractionJob {
                    id,
                    status: "failed",
                    provider_key: None,
                    provider_config_hash: None,
                    error_message: Some("LLM provider is not configured"),
                    failure_class: Some(failure_class_as_str(ExtractionFailureClass::Permanent)),
                    next_attempt_at: None,
                })
                .await?;
            warn!(
                job_id = %id,
                template_key = %job.template_key,
                attempt = job.attempt_count,
                failure_class = "permanent",
                latency_ms = 0,
                "extraction failed: provider not configured"
            );
            return Ok(false);
        };
        if !provider.enabled {
            self.store
                .finish_job(FinishExtractionJob {
                    id,
                    status: "skipped",
                    provider_key: None,
                    provider_config_hash: None,
                    error_message: Some("LLM provider is disabled"),
                    failure_class: None,
                    next_attempt_at: None,
                })
                .await?;
            return Ok(false);
        }
        let config_hash = provider.config_hash();
        let started = Instant::now();
        let result = extract(
            &self.http_client,
            &provider,
            &ProviderExtractionRequest {
                system_prompt: &template.system_prompt,
                user_content: &user_content(&document, &job.parameters),
                output_schema: &template.output_schema,
                max_output_tokens: template.max_output_tokens,
            },
        )
        .await;
        let latency = started.elapsed().as_millis() as i64;
        match result {
            Ok(result) => {
                self.store
                    .insert_attempt(ExtractionAttempt {
                        job_id: id,
                        provider_key: "llm",
                        provider_config_hash: &config_hash,
                        attempt_number: job.attempt_count,
                        status: "succeeded",
                        latency_ms: latency,
                        error_message: None,
                    })
                    .await?;
                let version_id = Uuid::new_v4();
                self.store
                    .publish_version(&ExtractionVersionInput {
                        id: version_id,
                        document_id: job.document_id,
                        template_key: &job.template_key,
                        template_version: job.template_version,
                        source_record_hash: &job.source_record_hash,
                        provider_key: "llm",
                        provider_config_hash: &config_hash,
                        model_name: result.model_name.as_deref(),
                        result_json: &result.result,
                    })
                    .await?;
                if let Err(error) = self
                    .publisher
                    .publish(&ExtractionPublication {
                        document_id: job.document_id,
                        group_id: document.group_id,
                        group_key: &document.group_key,
                        group_path: &document.group_path,
                        visibility: &document.visibility,
                        source_key: &document.source_key,
                        external_id: &document.external_id,
                        source_uri: &document.source_uri,
                        published_at: document.published_at,
                        updated_at: document.updated_at_source,
                        metadata_json: &document.metadata_json,
                        source_record_hash: &job.source_record_hash,
                        template_key: &job.template_key,
                        result_json: &result.result,
                    })
                    .await
                {
                    let msg = format!("metadata publish failed: {error:#}");
                    self.store
                        .finish_job(FinishExtractionJob {
                            id,
                            status: "failed",
                            provider_key: Some("llm"),
                            provider_config_hash: Some(&config_hash),
                            error_message: Some(&msg),
                            failure_class: Some(failure_class_as_str(
                                ExtractionFailureClass::Permanent,
                            )),
                            next_attempt_at: None,
                        })
                        .await?;
                    warn!(
                        job_id = %id,
                        template_key = %job.template_key,
                        attempt = job.attempt_count,
                        failure_class = "permanent",
                        latency_ms = latency,
                        "extraction publish failed"
                    );
                    return Ok(false);
                }
                self.store
                    .finish_job(FinishExtractionJob {
                        id,
                        status: "succeeded",
                        provider_key: Some("llm"),
                        provider_config_hash: Some(&config_hash),
                        error_message: None,
                        failure_class: None,
                        next_attempt_at: None,
                    })
                    .await?;
                info!(
                    job_id = %id,
                    template_key = %job.template_key,
                    attempt = job.attempt_count,
                    latency_ms = latency,
                    "extraction succeeded"
                );
                Ok(true)
            }
            Err(error) => {
                let message = format!("{error:#}");
                let failure_class = classify_error(&error);
                let failure_str = failure_class_as_str(failure_class);
                let attempt_status = if failure_class == ExtractionFailureClass::QuotaExceeded {
                    "quota_exceeded"
                } else {
                    "failed"
                };
                self.store
                    .insert_attempt(ExtractionAttempt {
                        job_id: id,
                        provider_key: "llm",
                        provider_config_hash: &config_hash,
                        attempt_number: job.attempt_count,
                        status: attempt_status,
                        latency_ms: latency,
                        error_message: Some(&message),
                    })
                    .await?;
                if failure_class == ExtractionFailureClass::Transient
                    && job.attempt_count < MAX_ATTEMPTS
                {
                    let delay = next_retry_delay(job.attempt_count);
                    let next_attempt_at = Utc::now()
                        + chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::seconds(5));
                    self.store
                        .finish_job(FinishExtractionJob {
                            id,
                            status: "queued",
                            provider_key: Some("llm"),
                            provider_config_hash: Some(&config_hash),
                            error_message: Some(&message),
                            failure_class: Some(failure_str),
                            next_attempt_at: Some(next_attempt_at),
                        })
                        .await?;
                    info!(
                        job_id = %id,
                        template_key = %job.template_key,
                        attempt = job.attempt_count,
                        failure_class = failure_str,
                        latency_ms = latency,
                        next_attempt_at = %next_attempt_at,
                        "extraction transient failure, scheduled retry"
                    );
                    // No explicit spawn: the run_pending loop sleeps until next_attempt_at (capped)
                    // and new enqueues wake via spawn_worker with lock handover.
                    Ok(false)
                } else {
                    self.store
                        .finish_job(FinishExtractionJob {
                            id,
                            status: "failed",
                            provider_key: Some("llm"),
                            provider_config_hash: Some(&config_hash),
                            error_message: Some(&message),
                            failure_class: Some(failure_str),
                            next_attempt_at: None,
                        })
                        .await?;
                    if failure_class == ExtractionFailureClass::QuotaExceeded {
                        warn!(
                            job_id = %id,
                            template_key = %job.template_key,
                            attempt = job.attempt_count,
                            failure_class = failure_str,
                            latency_ms = latency,
                            "extraction quota exceeded"
                        );
                    } else if failure_class == ExtractionFailureClass::Transient {
                        warn!(
                            job_id = %id,
                            template_key = %job.template_key,
                            attempt = job.attempt_count,
                            failure_class = failure_str,
                            latency_ms = latency,
                            "extraction transient failure exhausted retries"
                        );
                    } else {
                        warn!(
                            job_id = %id,
                            template_key = %job.template_key,
                            attempt = job.attempt_count,
                            failure_class = failure_str,
                            latency_ms = latency,
                            "extraction permanent failure"
                        );
                    }
                    Ok(false)
                }
            }
        }
    }
}

fn user_content(document: &crate::store::ExtractionDocument, parameters: &Value) -> String {
    let mut sections = Vec::new();
    if !parameters.is_null() && !parameters.as_object().is_some_and(|value| value.is_empty()) {
        sections.push(format!(
            "参数：{}",
            serde_json::to_string(parameters).unwrap_or_default()
        ));
    }
    if !document.title.is_empty() {
        sections.push(format!("标题：{}", document.title));
    }
    if let Some(summary) = document
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("摘要：{summary}"));
    }
    if !document.body_text.is_empty() {
        sections.push(format!("正文：{}", document.body_text));
    }
    sections.join("\n\n")
}
