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
use context69_secret_store::SecretStore;
use uuid::Uuid;

use crate::{
    EnqueueTranslation, TranslationChunkingBudget, TranslationCoordinator, TranslationDependencies,
    TranslationPublisher, TranslationReadiness,
    store::{TranslationStore, job_response, normalize_locale, normalize_locales},
};

#[derive(Clone)]
pub struct TranslationService {
    store: TranslationStore,
    http_client: reqwest::Client,
    publisher: Arc<dyn TranslationPublisher>,
    readiness: Arc<dyn TranslationReadiness>,
    /// The budget a conversion of this service chunks under when the caller
    /// passes no explicit snapshot.
    ///
    /// The running application leaves this `None`; a scoped service built from
    /// a task item's pinned snapshot carries that snapshot so a conversion
    /// reached through an existing helper still honors it.
    default_chunking: Option<TranslationChunkingBudget>,
}

impl TranslationService {
    pub fn new(dependencies: TranslationDependencies) -> Self {
        Self {
            store: TranslationStore::new(dependencies.pool),
            http_client: dependencies.http_client,
            publisher: dependencies.publisher,
            readiness: dependencies.readiness,
            default_chunking: None,
        }
    }

    /// A clone of this service whose conversions default to `budget`.
    #[must_use]
    pub fn with_default_chunking(mut self, budget: Option<TranslationChunkingBudget>) -> Self {
        self.default_chunking = budget;
        self
    }

    /// Binds the shared encrypted store this deployment configured.
    ///
    /// The shared `llm` provider key is written and resolved through it; until
    /// this is called the service runs in the unkeyed transition state.
    #[must_use]
    pub fn with_secret_store(mut self, store: SecretStore) -> Self {
        self.store = self.store.with_secret_store(store);
        self
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
        let chunking = self.operation_chunking();
        let job = self
            .store
            .retry_job(group_id, id)
            .await?
            .ok_or_else(|| DomainError::conflict("translation job is not retryable"))
            .map_err(anyhow::Error::from)?;
        self.run_job_blocking(job.id, chunking.as_ref()).await?;
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
        self.rebuild_document_with_chunking(
            group_id,
            document_id,
            request,
            self.default_chunking.as_ref(),
        )
        .await
    }

    /// Rebuilds a document's translations under an explicit chunk budget.
    ///
    /// `None` keeps the historical behavior: the publisher resolves the budget
    /// it currently considers active. A task item always passes its pinned
    /// snapshot, so a settings save cannot rechunk a task that started under an
    /// older budget.
    pub async fn rebuild_document_with_chunking(
        &self,
        group_id: i64,
        document_id: i64,
        request: &RebuildDocumentTranslationsRequest,
        chunking: Option<&TranslationChunkingBudget>,
    ) -> Result<TranslationJobsResponse> {
        let chunking = self.resolve_operation_chunking(chunking);
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
            .convert_with_chunking(
                EnqueueTranslation {
                    document_id,
                    directive,
                },
                chunking.as_ref(),
            )
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
        self.convert_with_chunking(input, self.default_chunking.as_ref())
            .await
    }
}

impl TranslationService {
    /// Resolves the budget of one translation operation.
    ///
    /// An explicit task/default snapshot always wins. Otherwise the publisher
    /// is asked once for the live budget, and the owned result is passed to
    /// every locale job of that operation, so a settings save cannot mix
    /// budgets inside one conversion.
    fn resolve_operation_chunking(
        &self,
        explicit: Option<&TranslationChunkingBudget>,
    ) -> Option<TranslationChunkingBudget> {
        explicit
            .cloned()
            .or_else(|| self.publisher.current_chunking())
    }

    /// The budget a conversion of this service would use right now.
    ///
    /// A file-level fan-out asks once before its first await and passes the
    /// owned value to every document conversion it starts.
    pub fn operation_chunking(&self) -> Option<TranslationChunkingBudget> {
        self.resolve_operation_chunking(self.default_chunking.as_ref())
    }

    /// Converts one document under an explicit operation budget.
    ///
    /// Resolution happens before the first await, so a settings save after the
    /// caller entered the operation cannot change its budget.
    pub async fn convert_with_chunking(
        &self,
        input: EnqueueTranslation,
        chunking: Option<&TranslationChunkingBudget>,
    ) -> Result<Vec<TranslationJobResponse>> {
        let chunking = self.resolve_operation_chunking(chunking);
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
            self.run_job_blocking(job.id, chunking.as_ref()).await?;
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
