use anyhow::{Context, Result, anyhow};
use uuid::Uuid;

use super::dependency_runtime::{
    dependency_is_transient, is_configuration_error, is_s3_error, is_s3_transient_error,
    is_transient_download_error,
};
use super::url_import_helpers::{
    clean_optional, hex_digest, job_metadata, job_translation, url_import_response,
};
use super::url_import_runtime::URL_IMPORT_LEASE_TTL_SECS;
use super::url_import_worker::{
    URL_IMPORT_PENDING_REQUEUE_SECS, URL_IMPORT_TRANSIENT_REQUEUE_SECS, UrlImportOutcome,
    UrlImportProgress,
};
use super::*;
use crate::{
    contracts::{
        ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata, LibraryIngestFailureStage,
        LibraryUrlImportJobResponse,
    },
    library_store::{NewLibraryUrlImportJob, UrlImportJobRecord},
};
use tracing::warn;

impl LibraryService {
    pub async fn import_url_in_project(
        &self,
        group: &crate::domain::GroupRecord,
        request: &ImportLibraryFileFromUrlRequest,
    ) -> Result<LibraryUrlImportJobResponse> {
        let url = remote_download::normalize_url(&request.url)?;
        if let Some(folder_id) = request.folder_id {
            self.store
                .get_folder_in_project(group.id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }
        if request
            .metadata
            .as_ref()
            .is_some_and(|value| !value.metadata_json.is_object())
        {
            return Err(anyhow!("metadata_json must be an object"));
        }
        let metadata = request.metadata.clone().unwrap_or_default();
        let translation = request.translation.clone().unwrap_or_default();
        let dedupe_identity = metadata.external_id.as_deref().unwrap_or(url.as_str());
        let dedupe_key = hex_digest(dedupe_identity.as_bytes());
        let job = self
            .store
            .create_url_import_job(&NewLibraryUrlImportJob {
                id: Uuid::new_v4(),
                group_id: group.id,
                visibility: group.visibility.as_str().to_string(),
                folder_id: request.folder_id,
                source_url: url.to_string(),
                dedupe_key,
                requested_filename: clean_optional(request.filename.as_deref()),
                requested_media_type: clean_optional(request.media_type.as_deref()),
                external_id: metadata.external_id,
                source_uri: metadata.source_uri,
                published_at: metadata.published_at,
                metadata_json: metadata.metadata_json,
                metadata_provided: request.metadata.is_some(),
                translation_provided: request.translation.is_some(),
                translation_source_locale: translation.source_locale,
                translation_target_locales: translation.target_locales,
            })
            .await?;
        self.url_import_runtime.notify();
        url_import_response(self, job).await
    }

    pub async fn get_url_import_job_in_project(
        &self,
        group_id: i64,
        job_id: Uuid,
    ) -> Result<LibraryUrlImportJobResponse> {
        let job = self
            .store
            .get_url_import_job_in_project(group_id, job_id)
            .await?
            .context("unknown URL import job")?;
        url_import_response(self, job).await
    }

    pub async fn retry_url_import_job_in_project(
        &self,
        group_id: i64,
        job_id: Uuid,
    ) -> Result<LibraryUrlImportJobResponse> {
        let retry_job_id = Uuid::new_v4();
        let job = self
            .store
            .retry_url_import_job(group_id, job_id, retry_job_id)
            .await?
            .context("URL import job is not failed and cannot be retried")?;
        self.url_import_runtime.notify();
        url_import_response(self, job).await
    }

    pub async fn resume_url_imports(&self) -> Result<()> {
        self.store.recover_expired_url_import_jobs().await?;
        self.url_import_runtime.start_workers(self.clone());
        self.url_import_runtime.notify();
        Ok(())
    }

    pub(super) async fn run_url_import(
        &self,
        job: &UrlImportJobRecord,
        lease_token: Uuid,
    ) -> Result<UrlImportOutcome> {
        let job_id = job.id;
        let result: IngestResult<UrlImportProgress> = if let Some(file_id) = job.file_id {
            self.retry_url_import_ingest(job, file_id, lease_token)
                .await
        } else {
            self.download_and_import(job, lease_token).await
        };
        match result {
            Ok(UrlImportProgress::Succeeded) => {
                let _ = self
                    .store
                    .finish_url_import_job(job_id, lease_token, "succeeded", None, None, None)
                    .await?;
                Ok(UrlImportOutcome::Succeeded)
            }
            Ok(UrlImportProgress::WaitingForIngest) => {
                let _ = self
                    .store
                    .release_url_import_ingesting_lease(job_id, lease_token)
                    .await?;
                Ok(UrlImportOutcome::Succeeded)
            }
            Ok(UrlImportProgress::Requeue) => {
                let _ = self
                    .store
                    .requeue_url_import_job(job_id, lease_token, URL_IMPORT_PENDING_REQUEUE_SECS)
                    .await?;
                Ok(UrlImportOutcome::Requeued)
            }
            Err(error) => {
                let error_message = error.error.to_string().to_ascii_lowercase();
                let download_retryable = error.stage == LibraryIngestFailureStage::Download
                    && is_transient_download_error(&error.error);
                let s3_retryable = is_s3_error(&error.error)
                    && (is_s3_transient_error(&error.error)
                        || error_message.contains("s3 dependency unavailable"));
                let dependency_retryable = error.dependency.is_some_and(|dependency| {
                    is_configuration_error(&error.error)
                        || dependency_is_transient(dependency, &error.error)
                });
                let retryable =
                    error.retryable || download_retryable || s3_retryable || dependency_retryable;
                if retryable {
                    if let Some(dependency) = error.dependency {
                        self.note_dependency_failure_with_lease(
                            dependency,
                            lease_token,
                            &error.error,
                        )
                        .await;
                    } else if s3_retryable {
                        self.note_dependency_failure_with_lease(
                            LibraryDependency::S3,
                            lease_token,
                            &error.error,
                        )
                        .await;
                    }
                    let _ = self
                        .store
                        .requeue_url_import_job(
                            job_id,
                            lease_token,
                            URL_IMPORT_TRANSIENT_REQUEUE_SECS,
                        )
                        .await?;
                    return Ok(UrlImportOutcome::Requeued);
                }
                let message = error.to_string();
                let code = message.split(':').next().unwrap_or("url_import_failed");
                let _ = self
                    .store
                    .finish_url_import_job(
                        job_id,
                        lease_token,
                        "failed",
                        Some(code),
                        Some(&message),
                        Some(error.stage),
                    )
                    .await?;
                Err(error.into())
            }
        }
    }

    async fn download_and_import(
        &self,
        job: &UrlImportJobRecord,
        lease_token: Uuid,
    ) -> IngestResult<UrlImportProgress> {
        let trusted_proxy_enabled = self
            .settings
            .trusted_proxy_enabled()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
        let limiter = self.url_import_runtime.limiter().ok_or_else(|| {
            IngestFailure::new(
                LibraryIngestFailureStage::Download,
                anyhow!("URL import rate limiter is unavailable; job remains queued"),
            )
            .retryable()
        })?;
        let downloaded = remote_download::download(
            &job.source_url,
            job.requested_filename.as_deref(),
            job.requested_media_type.as_deref(),
            self.max_upload_size_bytes,
            trusted_proxy_enabled,
            limiter.as_ref(),
        )
        .await
        .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Download, error))?;
        let sha = storage::hash_bytes(&downloaded.bytes);
        let existing = self
            .store
            .get_file_by_sha_in_project(job.group_id, &sha)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let metadata = if job.metadata_provided {
            Some(job_metadata(job))
        } else if existing.is_none() {
            Some(LibraryFileUploadMetadata {
                source_uri: Some(downloaded.url.to_string()),
                ..Default::default()
            })
        } else {
            None
        };
        let upload = self
            .upload_file_for_group_for_lease(
                job.group_id,
                UploadedLibraryFile {
                    folder_id: job.folder_id,
                    filename: downloaded.filename,
                    media_type: downloaded.media_type,
                    bytes: downloaded.bytes,
                    declared_sha256: Some(sha),
                    metadata,
                    translation: job_translation(job),
                },
                lease_token,
            )
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let marked = match self
            .store
            .mark_url_import_ingesting(
                job.id,
                lease_token,
                upload.file.file_id,
                Some(upload.job.job_id),
                URL_IMPORT_LEASE_TTL_SECS,
            )
            .await
        {
            Ok(marked) => marked,
            Err(error) => {
                self.rollback_uploaded_file(
                    &upload.file,
                    &upload.job,
                    upload.created_file,
                    upload.rollback,
                )
                .await;
                return Err(
                    IngestFailure::new(LibraryIngestFailureStage::Storage, error).retryable(),
                );
            }
        };
        if !marked {
            self.rollback_uploaded_file(
                &upload.file,
                &upload.job,
                upload.created_file,
                upload.rollback,
            )
            .await;
            return Ok(UrlImportProgress::Requeue);
        }
        self.finalize_uploaded_file(upload.rollback).await;
        Ok(UrlImportProgress::WaitingForIngest)
    }

    async fn retry_url_import_ingest(
        &self,
        job: &UrlImportJobRecord,
        file_id: Uuid,
        lease_token: Uuid,
    ) -> IngestResult<UrlImportProgress> {
        let file = self
            .store
            .get_file_in_project(job.group_id, file_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
            .context("URL import file no longer exists")
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        if let Some(ingest_job_id) = job.ingest_job_id {
            let ingest_job = self
                .store
                .get_job(ingest_job_id)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
                .context("URL import ingest job no longer exists")
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
            return match ingest_job.status {
                LibraryIngestStatus::Succeeded => Ok(UrlImportProgress::Succeeded),
                LibraryIngestStatus::Pending | LibraryIngestStatus::Running => {
                    Ok(UrlImportProgress::WaitingForIngest)
                }
                LibraryIngestStatus::Failed => Err(IngestFailure::new(
                    ingest_job
                        .failure_stage
                        .unwrap_or(LibraryIngestFailureStage::Other),
                    anyhow!(
                        ingest_job
                            .error_message
                            .unwrap_or_else(|| "ingest_failed".to_string())
                    ),
                )),
            };
        }

        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
        let ingest_job_id = Uuid::new_v4();
        self.store
            .create_job_with_options(
                ingest_job_id,
                file_id,
                super::uploads::requires_docling(kind),
                None,
            )
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let marked = match self
            .store
            .mark_url_import_ingesting(
                job.id,
                lease_token,
                file_id,
                Some(ingest_job_id),
                URL_IMPORT_LEASE_TTL_SECS,
            )
            .await
        {
            Ok(marked) => marked,
            Err(error) => {
                if let Err(cleanup_error) =
                    self.store.delete_pending_ingest_job(ingest_job_id).await
                {
                    warn!(
                        %ingest_job_id,
                        %cleanup_error,
                        "failed to remove URL retry ingest job after URL state update failure"
                    );
                }
                return Err(
                    IngestFailure::new(LibraryIngestFailureStage::Storage, error).retryable(),
                );
            }
        };
        if !marked {
            if let Err(error) = self.store.delete_pending_ingest_job(ingest_job_id).await {
                warn!(
                    %ingest_job_id,
                    %error,
                    "failed to remove unclaimed URL retry ingest job"
                );
            }
            return Ok(UrlImportProgress::Requeue);
        }
        Ok(UrlImportProgress::WaitingForIngest)
    }
}
