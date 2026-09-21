mod job_runner;

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use context69_contracts_extraction::{
    ExtractionHealthResponse, ExtractionJobResponse, ExtractionJobsResponse,
    ExtractionTemplateInput, ExtractionTemplateResponse, RebuildDocumentExtractionsRequest,
};
use uuid::Uuid;

use crate::{
    EnqueueExtraction, ExtractionCoordinator, ExtractionDependencies, ExtractionPublisher,
    ExtractionReadiness,
    store::{ExtractionStore, codec},
};

#[derive(Clone)]
pub struct ExtractionService {
    store: ExtractionStore,
    http_client: reqwest::Client,
    publisher: Arc<dyn ExtractionPublisher>,
    readiness: Arc<dyn ExtractionReadiness>,
}

impl ExtractionService {
    pub fn new(dependencies: ExtractionDependencies) -> Self {
        Self {
            store: ExtractionStore::new(dependencies.pool),
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

    pub async fn templates(&self) -> Result<Vec<ExtractionTemplateResponse>> {
        Ok(self
            .store
            .templates()
            .await?
            .into_iter()
            .map(codec::template_response)
            .collect())
    }

    pub async fn register_template(
        &self,
        input: &ExtractionTemplateInput,
    ) -> Result<ExtractionTemplateResponse> {
        self.store.register_template(input).await
    }

    pub async fn jobs(&self, group_id: i64, document_id: i64) -> Result<ExtractionJobsResponse> {
        let jobs = self
            .store
            .jobs_for_document(group_id, document_id)
            .await?
            .into_iter()
            .map(codec::job_response)
            .collect::<Result<Vec<_>>>()?;
        let latest_results = self
            .store
            .results_for_document(document_id)
            .await?
            .into_iter()
            .map(codec::result_response)
            .collect();
        Ok(ExtractionJobsResponse {
            jobs,
            latest_results,
        })
    }

    pub async fn retry(&self, group_id: i64, id: Uuid) -> Result<ExtractionJobResponse> {
        let job = self
            .store
            .retry_job(group_id, id)
            .await?
            .context("extraction job is not retryable")?;
        self.run_job_blocking(group_id, job.id).await
    }

    pub async fn health(&self) -> Result<ExtractionHealthResponse> {
        self.store.health().await
    }

    pub async fn rebuild(
        &self,
        group_id: i64,
        document_id: i64,
        request: &RebuildDocumentExtractionsRequest,
    ) -> Result<ExtractionJobsResponse> {
        let document = self.store.document(document_id).await?;
        if document.group_id != group_id {
            return Err(anyhow!("extraction document not found"));
        }
        let templates = self.store.templates().await?;
        let mut jobs = Vec::new();
        for template in templates {
            if template.enabled
                && (request.template_keys.is_empty()
                    || request.template_keys.contains(&template.template_key))
            {
                let job = self
                    .store
                    .insert_job(
                        document_id,
                        &template,
                        &document.record_hash,
                        &serde_json::json!({}),
                    )
                    .await?;
                jobs.push(self.run_job_blocking(group_id, job.id).await?);
            }
        }
        Ok(ExtractionJobsResponse {
            jobs,
            latest_results: Vec::new(),
        })
    }
}

#[async_trait]
impl ExtractionCoordinator for ExtractionService {
    /// Blocking conversion: insert or reuse the document's extraction job and
    /// run it to completion before returning. There is no background worker,
    /// so a job can never be left for a later dispatcher re-claim.
    async fn convert(&self, input: EnqueueExtraction) -> Result<Vec<ExtractionJobResponse>> {
        let document = self.store.document(input.document_id).await?;
        let Some(template) = self.store.template(&input.directive.template_key).await? else {
            return Err(anyhow!(
                "extraction template {} not found",
                input.directive.template_key
            ));
        };
        if !template.enabled {
            return Ok(Vec::new());
        }
        let job = self
            .store
            .insert_job(
                input.document_id,
                &template,
                &document.record_hash,
                &input.directive.parameters,
            )
            .await?;
        let response = self.run_job_blocking(document.group_id, job.id).await?;
        Ok(vec![response])
    }
}
