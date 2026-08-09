use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::ExtractionService;
use crate::{
    ExtractionPublication,
    providers::{ProviderExtractionRequest, extract},
    store::{ExtractionAttempt, ExtractionVersionInput, FinishExtractionJob},
};

const READINESS_POLL_INTERVAL: Duration = Duration::from_secs(5);

impl ExtractionService {
    pub(super) async fn run_pending(&self) -> Result<()> {
        let Ok(_guard) = self.worker_lock.try_lock() else {
            return Ok(());
        };
        loop {
            let ids = self.store.pending_ids().await?;
            if ids.is_empty() {
                return Ok(());
            }
            if !self.readiness.is_ready().await? {
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
            for task in tasks {
                if let Err(error) = task.await? {
                    warn!(%error, "extraction job failed");
                }
            }
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
                })
                .await?;
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
                })
                .await?;
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
                })
                .await?;
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
                    self.store
                        .finish_job(FinishExtractionJob {
                            id,
                            status: "failed",
                            provider_key: Some("llm"),
                            provider_config_hash: Some(&config_hash),
                            error_message: Some(&format!("metadata publish failed: {error:#}")),
                        })
                        .await?;
                    return Ok(false);
                }
                self.store
                    .finish_job(FinishExtractionJob {
                        id,
                        status: "succeeded",
                        provider_key: Some("llm"),
                        provider_config_hash: Some(&config_hash),
                        error_message: None,
                    })
                    .await?;
                Ok(true)
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.store
                    .insert_attempt(ExtractionAttempt {
                        job_id: id,
                        provider_key: "llm",
                        provider_config_hash: &config_hash,
                        attempt_number: job.attempt_count,
                        status: "failed",
                        latency_ms: latency,
                        error_message: Some(&message),
                    })
                    .await?;
                self.store
                    .finish_job(FinishExtractionJob {
                        id,
                        status: "failed",
                        provider_key: Some("llm"),
                        provider_config_hash: Some(&config_hash),
                        error_message: Some(&message),
                    })
                    .await?;
                Ok(false)
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
