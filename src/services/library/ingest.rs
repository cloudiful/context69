use anyhow::{Context, Result, anyhow};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{info, warn};
use uuid::Uuid;

use super::dependency_runtime::{
    dependency_is_transient, ingest_heartbeat_interval, ingest_lease_ttl_secs,
    is_configuration_error, is_s3_error, probe_lease_ttl_secs,
};
use super::*;

impl LibraryService {
    pub(super) async fn run_ingest_claim(
        &self,
        claim: crate::library_store::IngestClaim,
        probe_dependencies: Vec<LibraryDependency>,
    ) -> Result<IngestClaimOutcome> {
        let file_id = claim.file_id;
        let job_id = claim.job_id;
        let _permit = self.ingest_semaphore.acquire().await?;
        let file_status = self
            .store
            .update_file_status(file_id, LibraryIngestStatus::Running, None, false)
            .await?;
        if file_status.is_none() {
            self.store
                .release_ingest_job(job_id, claim.lease_token)
                .await?;
            return Ok(IngestClaimOutcome::Requeued);
        }
        let heartbeat = self.spawn_ingest_heartbeat(job_id, claim.lease_token, probe_dependencies);
        let mut ingest_artifacts_may_exist = false;

        let result: IngestResult<()> = async {
            let file = self
                .store
                .get_file(file_id)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
                .ok_or_else(|| {
                    IngestFailure::new(
                        LibraryIngestFailureStage::Storage,
                        anyhow!("unknown file {file_id}"),
                    )
                })?;
            let bytes = self
                .read_active_storage_for_lease(&file.storage_rel_path, claim.lease_token)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
                .with_context(|| format!("stored file not found for file {file_id}"))
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
            info!(
                file_id = %file_id,
                file_bytes = bytes.len(),
                "library ingest input loaded"
            );

            let kind = storage::detect_file_kind(&file.filename, &file.media_type)
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
            self.runtime().map_err(|error| {
                IngestFailure::new(LibraryIngestFailureStage::Other, error)
                    .with_dependency(LibraryDependency::EmbeddingVector)
            })?;
            let sections = if let Some(payload) = claim.section_payload.clone() {
                serde_json::from_value::<Vec<IngestSection>>(payload).map_err(|error| {
                    IngestFailure::new(
                        LibraryIngestFailureStage::Parsing,
                        anyhow!("invalid persisted ingest section payload: {error}"),
                    )
                })?
            } else {
                match kind {
                    LibraryFileKind::Pdf => {
                        let task_timeout = self.docling_task_timeout().await.map_err(|error| {
                            IngestFailure::new(LibraryIngestFailureStage::Docling, error)
                        })?;
                        timeout(task_timeout, self.ingest_pdf(&file, &bytes))
                            .await
                            .map_err(|error| {
                                IngestFailure::new(
                                    LibraryIngestFailureStage::Docling,
                                    anyhow!("docling conversion timed out: {error}"),
                                )
                            })??
                    }
                    LibraryFileKind::Docx => {
                        let task_timeout = self.docling_task_timeout().await.map_err(|error| {
                            IngestFailure::new(LibraryIngestFailureStage::Docling, error)
                        })?;
                        timeout(task_timeout, self.ingest_docx(&file, &bytes))
                            .await
                            .map_err(|error| {
                                IngestFailure::new(
                                    LibraryIngestFailureStage::Docling,
                                    anyhow!("docling conversion timed out: {error}"),
                                )
                            })??
                    }
                    LibraryFileKind::Xlsx => {
                        let task_timeout = self.docling_task_timeout().await.map_err(|error| {
                            IngestFailure::new(LibraryIngestFailureStage::Docling, error)
                        })?;
                        timeout(task_timeout, self.ingest_xlsx(&file, &bytes))
                            .await
                            .map_err(|error| {
                                IngestFailure::new(
                                    LibraryIngestFailureStage::Docling,
                                    anyhow!("docling conversion timed out: {error}"),
                                )
                            })??
                    }
                    LibraryFileKind::PlainText => self.ingest_text(&file, &bytes).await?,
                }
            };
            drop(bytes);
            let prepared_sections = self.prepare_sections(&file, sections).await?;
            ingest_artifacts_may_exist = true;
            self.persist_sections(&file, prepared_sections).await
        }
        .await;

        let final_result: Result<IngestClaimOutcome> = async {
            match result {
                Ok(()) => {
                    let Some(_) = self
                        .store
                        .finish_ingest_job(
                            job_id,
                            claim.lease_token,
                            LibraryIngestStatus::Succeeded,
                            None,
                            None,
                        )
                        .await?
                    else {
                        return Ok(IngestClaimOutcome::Requeued);
                    };
                    for dependency in claim_dependencies(&claim) {
                        self.note_dependency_success(dependency, claim.lease_token)
                            .await;
                    }
                    if let Err(error) = self.enqueue_file_translations(file_id).await {
                        warn!(
                            file_id = %file_id,
                            job_id = %job_id,
                            %error,
                            "library ingest succeeded but translation jobs could not be queued"
                        );
                    }
                    info!(file_id = %file_id, job_id = %job_id, "library ingest succeeded");
                    Ok(IngestClaimOutcome::Succeeded)
                }
                Err(mut error) => {
                    let inferred_dependency =
                        infer_dependency(&error, &claim, self.runtime.is_some());
                    if error.dependency.is_none() {
                        error.dependency = inferred_dependency;
                    }
                    if let Some(dependency) = error.dependency {
                        error.retryable |= dependency_is_transient(dependency, &error.error)
                            || is_configuration_error(&error.error);
                    }

                    if (error.retryable || ingest_artifacts_may_exist)
                        && let Err(cleanup_error) = self.cleanup_ingest_artifacts(file_id).await
                    {
                        warn!(file_id = %file_id, error = %cleanup_error, "failed to clean library ingest artifacts; keeping job pending");
                        if cleanup_error
                            .to_string()
                            .to_ascii_lowercase()
                            .contains("qdrant")
                        {
                            self.note_dependency_failure_with_lease(
                                LibraryDependency::EmbeddingVector,
                                claim.lease_token,
                                &cleanup_error,
                            )
                            .await;
                        }
                        self.store
                            .release_ingest_job(job_id, claim.lease_token)
                            .await?;
                        return Ok(IngestClaimOutcome::Requeued);
                    }

                    let message = error.to_string();
                    if error.retryable {
                        if let Some(dependency) = error.dependency {
                            self.note_dependency_failure_with_lease(
                                dependency,
                                claim.lease_token,
                                &error.error,
                            )
                            .await;
                        }
                        self.store
                            .release_ingest_job(job_id, claim.lease_token)
                            .await?;
                        return Ok(IngestClaimOutcome::Requeued);
                    }

                    self.store
                        .finish_ingest_job(
                            job_id,
                            claim.lease_token,
                            LibraryIngestStatus::Failed,
                            Some(error.stage),
                            Some(&message),
                        )
                        .await?;
                    Err(anyhow::Error::new(error))
                }
            }
        }
        .await;
        heartbeat.abort();
        let _ = heartbeat.await;
        final_result
    }

    fn spawn_ingest_heartbeat(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        probe_dependencies: Vec<LibraryDependency>,
    ) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(ingest_heartbeat_interval());
            loop {
                interval.tick().await;
                match service
                    .store
                    .heartbeat_ingest_job(job_id, lease_token, ingest_lease_ttl_secs())
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        warn!(%job_id, %error, "failed to heartbeat library ingest job");
                    }
                }
                for dependency in &probe_dependencies {
                    if let Err(error) = service
                        .store
                        .renew_dependency_probe(
                            dependency.as_str(),
                            lease_token,
                            probe_lease_ttl_secs(),
                        )
                        .await
                    {
                        warn!(
                            %job_id,
                            dependency = dependency.as_str(),
                            %error,
                            "failed to heartbeat library dependency probe"
                        );
                    }
                }
            }
        })
    }
}

fn claim_dependencies(claim: &crate::library_store::IngestClaim) -> Vec<LibraryDependency> {
    let mut dependencies = vec![LibraryDependency::EmbeddingVector];
    if claim.requires_docling {
        dependencies.push(LibraryDependency::Docling);
    }
    if claim.storage_backend == "s3" {
        dependencies.push(LibraryDependency::S3);
    }
    dependencies
}

fn infer_dependency(
    failure: &IngestFailure,
    claim: &crate::library_store::IngestClaim,
    runtime_available: bool,
) -> Option<LibraryDependency> {
    if is_s3_error(&failure.error) {
        return Some(LibraryDependency::S3);
    }
    if failure.stage == LibraryIngestFailureStage::Docling || claim.requires_docling {
        let message = failure.error.to_string().to_ascii_lowercase();
        if failure.stage == LibraryIngestFailureStage::Docling
            || message.contains("docling")
            || message.contains("conversion")
        {
            return Some(LibraryDependency::Docling);
        }
    }
    let message = failure.error.to_string().to_ascii_lowercase();
    if failure.stage == LibraryIngestFailureStage::Embedding
        || (failure.stage == LibraryIngestFailureStage::Indexing
            && (message.contains("qdrant") || message.contains("embedding")))
        || (!runtime_available && failure.stage == LibraryIngestFailureStage::Other)
    {
        return Some(LibraryDependency::EmbeddingVector);
    }
    None
}
