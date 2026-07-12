use std::time::Instant;

use anyhow::Result;
use context69_contracts::TranslationGlossaryEntry;
use tracing::warn;
use uuid::Uuid;

use super::TranslationService;
use crate::{
    TranslationPublication,
    providers::{ProviderTranslationRequest, source_character_count, translate},
    segmenter::{segment_document, translatable_segments, translated_document},
    store::{
        FinishJob, TranslationAttempt, TranslationDocument, TranslationJobRecord,
        TranslationVersionInput,
    },
};

struct PublishedText<'a> {
    source_locale: Option<&'a str>,
    provider_key: &'a str,
    config_hash: &'a str,
    model_name: Option<&'a str>,
    title: &'a str,
    summary: Option<&'a str>,
    body: &'a str,
}

impl TranslationService {
    pub(super) async fn run_pending(&self) -> Result<()> {
        let Ok(_guard) = self.worker_lock.try_lock() else {
            return Ok(());
        };
        let ids = self.store.pending_ids().await?;
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
                warn!(%error, "translation job failed");
            }
        }
        Ok(())
    }

    async fn run_job(&self, id: Uuid) -> Result<()> {
        let Some(job) = self.store.claim_job(id).await? else {
            return Ok(());
        };
        let document = self.store.document(job.document_id).await?;
        if document.record_hash != job.source_record_hash {
            self.store
                .finish_job(FinishJob {
                    id,
                    status: "failed",
                    source_locale: None,
                    provider_key: None,
                    provider_config_hash: None,
                    character_count: 0,
                    error_message: Some("source document changed"),
                })
                .await?;
            return Ok(());
        }
        let detected = job
            .requested_source_locale
            .clone()
            .or_else(|| detect_locale(&document.title, &document.body_text));
        if detected
            .as_deref()
            .is_some_and(|source| same_language(source, &job.target_locale))
        {
            self.store
                .finish_job(FinishJob {
                    id,
                    status: "skipped",
                    source_locale: detected.as_deref(),
                    provider_key: None,
                    provider_config_hash: None,
                    character_count: 0,
                    error_message: None,
                })
                .await?;
            return Ok(());
        }
        let group = self.store.group_settings(document.group_id).await?;
        let glossary = serde_json::from_value::<Vec<TranslationGlossaryEntry>>(group.glossary)?;
        let segmented = segment_document(
            &document.title,
            document.summary.as_deref(),
            &document.body_text,
        );
        let segments = translatable_segments(&segmented);
        let character_count = source_character_count(&segments);
        let mut errors = Vec::new();
        let mut quota_only = true;
        for (attempt_index, provider) in self
            .store
            .providers()
            .await?
            .into_iter()
            .filter(|provider| provider.enabled)
            .enumerate()
        {
            let config_hash = provider.config_hash();
            if !self.store.reserve_usage(&provider, character_count).await? {
                self.store
                    .insert_attempt(TranslationAttempt {
                        job_id: id,
                        provider_key: &provider.provider_key,
                        provider_config_hash: &config_hash,
                        attempt_number: attempt_index as i32 + 1,
                        status: "quota_exceeded",
                        character_count,
                        latency_ms: 0,
                        error_message: Some("monthly character limit exceeded"),
                    })
                    .await?;
                errors.push(format!("{}: quota exceeded", provider.provider_key));
                continue;
            }
            quota_only = false;
            let started = Instant::now();
            let result = translate(
                &self.http_client,
                &provider,
                &ProviderTranslationRequest {
                    source_locale: detected.as_deref(),
                    target_locale: &job.target_locale,
                    segments: &segments,
                    glossary: &glossary,
                },
            )
            .await;
            let latency = started.elapsed().as_millis() as i64;
            match result {
                Ok(result) => {
                    self.store
                        .insert_attempt(TranslationAttempt {
                            job_id: id,
                            provider_key: &provider.provider_key,
                            provider_config_hash: &config_hash,
                            attempt_number: attempt_index as i32 + 1,
                            status: "succeeded",
                            character_count,
                            latency_ms: latency,
                            error_message: None,
                        })
                        .await?;
                    let (title, summary, body) =
                        translated_document(&segmented, &result.translations)?;
                    self.publish_translation(
                        &job,
                        &document,
                        PublishedText {
                            source_locale: detected.as_deref(),
                            provider_key: &provider.provider_key,
                            config_hash: &config_hash,
                            model_name: result.model_name.as_deref(),
                            title: &title,
                            summary: summary.as_deref(),
                            body: &body,
                        },
                    )
                    .await?;
                    self.store
                        .finish_job(FinishJob {
                            id,
                            status: "succeeded",
                            source_locale: detected.as_deref(),
                            provider_key: Some(&provider.provider_key),
                            provider_config_hash: Some(&config_hash),
                            character_count,
                            error_message: None,
                        })
                        .await?;
                    return Ok(());
                }
                Err(error) => {
                    let message = error.to_string();
                    self.store
                        .insert_attempt(TranslationAttempt {
                            job_id: id,
                            provider_key: &provider.provider_key,
                            provider_config_hash: &config_hash,
                            attempt_number: attempt_index as i32 + 1,
                            status: "failed",
                            character_count,
                            latency_ms: latency,
                            error_message: Some(&message),
                        })
                        .await?;
                    errors.push(format!("{}: {message}", provider.provider_key));
                }
            }
        }
        let status = if quota_only {
            "quota_exceeded"
        } else {
            "failed"
        };
        let message = if errors.is_empty() {
            "no enabled translation provider".to_string()
        } else {
            errors.join("; ")
        };
        self.store
            .finish_job(FinishJob {
                id,
                status,
                source_locale: detected.as_deref(),
                provider_key: None,
                provider_config_hash: None,
                character_count,
                error_message: Some(&message),
            })
            .await?;
        Ok(())
    }

    async fn publish_translation(
        &self,
        job: &TranslationJobRecord,
        document: &TranslationDocument,
        text: PublishedText<'_>,
    ) -> Result<()> {
        let old_ids = self
            .store
            .current_translation_chunk_ids(job.document_id, &job.target_locale)
            .await?;
        let published = self
            .publisher
            .publish(
                &old_ids,
                TranslationPublication {
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
                    source_record_hash: &document.record_hash,
                    target_locale: &job.target_locale,
                    source_locale: text.source_locale,
                    provider_key: text.provider_key,
                    title: text.title,
                    summary: text.summary,
                    body_text: text.body,
                },
            )
            .await?;
        let chunks = published
            .iter()
            .map(|chunk| (chunk.chunk_id, chunk.chunk_text.clone()))
            .collect::<Vec<_>>();
        let input = TranslationVersionInput {
            id: Uuid::new_v4(),
            document_id: job.document_id,
            target_locale: &job.target_locale,
            source_locale: text.source_locale,
            source_record_hash: &job.source_record_hash,
            provider_key: text.provider_key,
            provider_config_hash: text.config_hash,
            model_name: text.model_name,
            title: text.title,
            summary: text.summary,
            body_text: text.body,
        };
        if let Err(error) = self.store.publish_version(&input, &chunks).await {
            let new_ids = published
                .iter()
                .map(|chunk| chunk.chunk_id)
                .collect::<Vec<_>>();
            self.publisher.delete(&new_ids).await?;
            return Err(error);
        }
        Ok(())
    }
}

fn detect_locale(title: &str, body: &str) -> Option<String> {
    let info = whatlang::detect(&format!("{title}\n{body}"))?;
    if info.confidence() < 0.65 {
        return None;
    }
    Some(match info.lang() {
        whatlang::Lang::Eng => "en".to_string(),
        whatlang::Lang::Cmn => "zh".to_string(),
        language => language.code().to_string(),
    })
}

fn same_language(left: &str, right: &str) -> bool {
    left.split('-').next().map(str::to_ascii_lowercase)
        == right.split('-').next().map(str::to_ascii_lowercase)
}
