mod job_runner;

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use context69_contracts_core::errors::DomainError;
use context69_contracts_translation::{
    GroupTranslationSettingsResponse, RebuildDocumentTranslationsRequest, TranslationDirective,
    TranslationJobResponse, TranslationJobsResponse, TranslationProviderPageResponse,
    TranslationSettingsResponse, UpdateGroupTranslationSettingsRequest,
    UpdateTranslationSettingsRequest,
};
use uuid::Uuid;

use crate::{
    EnqueueTranslation, TranslationCoordinator, TranslationDependencies, TranslationPublisher,
    TranslationReadiness,
    store::{TranslationStore, job_response, normalize_locale, normalize_locales},
};

#[derive(Clone)]
pub struct TranslationService {
    store: TranslationStore,
    http_client: reqwest::Client,
    publisher: Arc<dyn TranslationPublisher>,
    readiness: Arc<dyn TranslationReadiness>,
}

impl TranslationService {
    pub fn new(dependencies: TranslationDependencies) -> Self {
        Self {
            store: TranslationStore::new(dependencies.pool),
            http_client: dependencies.http_client,
            publisher: dependencies.publisher,
            readiness: dependencies.readiness,
        }
    }

    /// Requeue jobs interrupted by a restart. Jobs are drained inside the
    /// blocking caller that owns them, so there is no worker to resume.
    pub async fn resume(&self) -> Result<()> {
        self.store.reset_interrupted().await
    }

    pub async fn settings(&self) -> Result<TranslationSettingsResponse> {
        self.store.settings().await
    }

    pub async fn provider_page(
        &self,
        page: u32,
        page_size: u32,
    ) -> Result<TranslationProviderPageResponse> {
        self.store.provider_page(page, page_size).await
    }

    pub async fn update_settings(
        &self,
        request: &UpdateTranslationSettingsRequest,
    ) -> Result<TranslationSettingsResponse> {
        self.store.update_settings(request).await
    }

    pub async fn group_settings(&self, group_id: i64) -> Result<GroupTranslationSettingsResponse> {
        self.store.group_settings_response(group_id).await
    }

    pub async fn update_group_settings(
        &self,
        group_id: i64,
        request: &UpdateGroupTranslationSettingsRequest,
    ) -> Result<GroupTranslationSettingsResponse> {
        self.store.update_group_settings(group_id, request).await
    }

    pub async fn job(&self, group_id: i64, id: Uuid) -> Result<TranslationJobResponse> {
        job_response(
            self.store
                .job_in_group(group_id, id)
                .await?
                .ok_or_else(|| DomainError::not_found("translation job not found"))
                .map_err(anyhow::Error::from)?,
        )
    }

    pub async fn jobs_for_document(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<TranslationJobsResponse> {
        let jobs = self
            .store
            .jobs_for_document(group_id, document_id)
            .await?
            .into_iter()
            .map(job_response)
            .collect::<Result<Vec<_>>>()?;
        Ok(TranslationJobsResponse { jobs })
    }

    pub async fn retry_job(&self, group_id: i64, id: Uuid) -> Result<TranslationJobResponse> {
        let job = self
            .store
            .retry_job(group_id, id)
            .await?
            .ok_or_else(|| DomainError::conflict("translation job is not retryable"))
            .map_err(anyhow::Error::from)?;
        self.run_job_blocking(job.id).await?;
        let record = self
            .store
            .job_in_group(group_id, job.id)
            .await?
            .ok_or_else(|| DomainError::not_found("translation job not found"))?;
        job_response(record)
    }

    pub async fn rebuild_document(
        &self,
        group_id: i64,
        document_id: i64,
        request: &RebuildDocumentTranslationsRequest,
    ) -> Result<TranslationJobsResponse> {
        let document = self.store.document(document_id).await?;
        if document.group_id != group_id {
            return Err(DomainError::not_found("translation document not found").into());
        }
        let directive = if request.target_locales.is_empty() {
            None
        } else {
            Some(TranslationDirective {
                source_locale: None,
                target_locales: request.target_locales.clone(),
            })
        };
        let jobs = self
            .convert(EnqueueTranslation {
                document_id,
                directive,
            })
            .await?;
        Ok(TranslationJobsResponse { jobs })
    }
}

#[async_trait]
impl TranslationCoordinator for TranslationService {
    /// Blocking conversion: reuse or insert the document's translation jobs
    /// and run them to completion before returning. There is no background
    /// worker, so a job can never be left for a later dispatcher re-claim.
    async fn convert(&self, input: EnqueueTranslation) -> Result<Vec<TranslationJobResponse>> {
        let document = self.store.document(input.document_id).await?;
        let group = self.store.group_settings(document.group_id).await?;
        let (source_locale, target_locales) = match input.directive {
            Some(directive) => (
                directive
                    .source_locale
                    .as_deref()
                    .map(normalize_locale)
                    .transpose()?,
                normalize_locales(&directive.target_locales)?,
            ),
            None if group.enabled => (group.source_locale, group.default_target_locales),
            None => return Ok(Vec::new()),
        };
        let mut jobs = Vec::new();
        for target_locale in target_locales {
            let job = self
                .store
                .insert_job(
                    input.document_id,
                    &target_locale,
                    source_locale.as_deref(),
                    &document.record_hash,
                )
                .await?;
            self.run_job_blocking(job.id).await?;
            let record = self
                .store
                .job_in_group(document.group_id, job.id)
                .await?
                .ok_or_else(|| DomainError::not_found("translation job not found"))?;
            if matches!(record.status.as_str(), "queued" | "running") {
                return Err(anyhow!("translation job {} did not complete", record.id));
            }
            jobs.push(job_response(record)?);
        }
        Ok(jobs)
    }
}
