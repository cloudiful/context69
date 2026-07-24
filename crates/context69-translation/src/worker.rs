mod job_runner;

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use context69_contracts::{
    GroupTranslationSettingsResponse, RebuildDocumentTranslationsRequest, TranslationDirective,
    TranslationJobResponse, TranslationJobsResponse, TranslationProviderPageResponse,
    TranslationSettingsResponse, UpdateGroupTranslationSettingsRequest,
    UpdateTranslationSettingsRequest,
};
use tokio::sync::{Mutex, Semaphore};
use tracing::error;
use uuid::Uuid;

use crate::{
    EnqueueTranslation, TranslationCoordinator, TranslationDependencies, TranslationPublisher,
    store::{TranslationStore, job_response, normalize_locale, normalize_locales},
};

#[derive(Clone)]
pub struct TranslationService {
    store: TranslationStore,
    http_client: reqwest::Client,
    publisher: Arc<dyn TranslationPublisher>,
    semaphore: Arc<Semaphore>,
    worker_lock: Arc<Mutex<()>>,
}

impl TranslationService {
    pub fn new(dependencies: TranslationDependencies) -> Self {
        Self {
            store: TranslationStore::new(dependencies.pool),
            http_client: dependencies.http_client,
            publisher: dependencies.publisher,
            semaphore: Arc::new(Semaphore::new(dependencies.concurrency.max(1))),
            worker_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn resume(&self) -> Result<()> {
        self.store.reset_interrupted().await?;
        self.spawn_worker();
        Ok(())
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
                .context("translation job not found")?,
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
            .context("translation job is not retryable")?;
        self.spawn_worker();
        job_response(job)
    }

    pub async fn rebuild_document(
        &self,
        group_id: i64,
        document_id: i64,
        request: &RebuildDocumentTranslationsRequest,
    ) -> Result<TranslationJobsResponse> {
        let document = self.store.document(document_id).await?;
        if document.group_id != group_id {
            return Err(anyhow!("translation document not found"));
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
            .enqueue(EnqueueTranslation {
                document_id,
                directive,
            })
            .await?;
        Ok(TranslationJobsResponse { jobs })
    }

    fn spawn_worker(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run_pending().await {
                error!(%error, "translation worker failed");
            }
        });
    }
}

#[async_trait]
impl TranslationCoordinator for TranslationService {
    async fn enqueue(&self, input: EnqueueTranslation) -> Result<Vec<TranslationJobResponse>> {
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
            jobs.push(job_response(
                self.store
                    .insert_job(
                        input.document_id,
                        &target_locale,
                        source_locale.as_deref(),
                        &document.record_hash,
                    )
                    .await?,
            )?);
        }
        if !jobs.is_empty() {
            self.spawn_worker();
        }
        Ok(jobs)
    }
}
