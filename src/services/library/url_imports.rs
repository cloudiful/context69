use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::url_import_runtime::{
    URL_IMPORT_HEARTBEAT_INTERVAL, URL_IMPORT_LEASE_TTL_SECS, UrlImportRuntime,
};
use super::*;
use crate::{
    contracts::{
        ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata, LibraryIngestFailureStage,
        LibraryUrlImportJobResponse, LibraryUrlImportStatus,
    },
    library_store::{NewLibraryUrlImportJob, UrlImportJobRecord},
};

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
        self.url_import_response(job).await
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
        self.url_import_response(job).await
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
        self.url_import_response(job).await
    }

    pub async fn resume_url_imports(&self) -> Result<()> {
        self.store.recover_expired_url_import_jobs().await?;
        self.url_import_runtime.start_workers(self.clone());
        self.url_import_runtime.notify();
        Ok(())
    }

    pub(super) async fn run_url_import_worker(&self, runtime: UrlImportRuntime, worker_id: usize) {
        loop {
            if let Err(error) = self.store.recover_expired_url_import_jobs().await {
                warn!(worker_id, %error, "failed to recover expired URL import leases");
            }
            let lease_token = Uuid::new_v4();
            let job = match self
                .store
                .claim_next_url_import_job(lease_token, URL_IMPORT_LEASE_TTL_SECS)
                .await
            {
                Ok(Some(job)) => job,
                Ok(None) => {
                    tokio::select! {
                        _ = runtime.wait_for_work() => {}
                        _ = tokio::time::sleep(UrlImportRuntime::poll_interval()) => {}
                    }
                    continue;
                }
                Err(error) => {
                    warn!(worker_id, %error, "failed to claim URL import job");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            let heartbeat = self.spawn_url_import_heartbeat(job.id, lease_token);
            if let Err(error) = self.run_url_import(&job, lease_token).await {
                warn!(worker_id, job_id = %job.id, %error, "URL import failed");
            }
            heartbeat.abort();
        }
    }

    fn spawn_url_import_heartbeat(&self, job_id: Uuid, lease_token: Uuid) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(URL_IMPORT_HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;
                match service
                    .store
                    .heartbeat_url_import_job(job_id, lease_token, URL_IMPORT_LEASE_TTL_SECS)
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        warn!(%job_id, %error, "failed to heartbeat URL import job");
                    }
                }
            }
        })
    }

    async fn run_url_import(&self, job: &UrlImportJobRecord, lease_token: Uuid) -> Result<()> {
        let job_id = job.id;
        let result: IngestResult<()> = if let Some(file_id) = job.file_id {
            self.retry_url_import_ingest(job, file_id, lease_token)
                .await
        } else {
            self.download_and_import(job, lease_token).await
        };
        match result {
            Ok(()) => {
                let _ = self
                    .store
                    .finish_url_import_job(job_id, lease_token, "succeeded", None, None, None)
                    .await?;
                Ok(())
            }
            Err(error) => {
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
    ) -> IngestResult<()> {
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
        let (file, ingest_job) = self
            .upload_file_for_group(
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
            )
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let marked = self
            .store
            .mark_url_import_ingesting(
                job.id,
                lease_token,
                file.file_id,
                Some(ingest_job.job_id),
                URL_IMPORT_LEASE_TTL_SECS,
            )
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        if !marked {
            let message = "parent URL import job is no longer active";
            let child_updated = self
                .store
                .update_job_status(
                    ingest_job.job_id,
                    LibraryIngestStatus::Failed,
                    None,
                    Some(LibraryIngestFailureStage::Other),
                    Some(message),
                    false,
                    true,
                )
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
            if child_updated.is_some() {
                self.store
                    .update_file_status(
                        file.file_id,
                        LibraryIngestStatus::Failed,
                        Some(message),
                        false,
                    )
                    .await
                    .map_err(|error| {
                        IngestFailure::new(LibraryIngestFailureStage::Storage, error)
                    })?;
            }
            return Ok(());
        }
        self.await_ingest(ingest_job.job_id).await
    }

    async fn retry_url_import_ingest(
        &self,
        job: &UrlImportJobRecord,
        file_id: Uuid,
        lease_token: Uuid,
    ) -> IngestResult<()> {
        let file = self
            .store
            .get_file_in_project(job.group_id, file_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
            .context("URL import file no longer exists")
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
        let ingest_job_id = Uuid::new_v4();
        self.store
            .create_job(ingest_job_id, file_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let marked = self
            .store
            .mark_url_import_ingesting(
                job.id,
                lease_token,
                file_id,
                Some(ingest_job_id),
                URL_IMPORT_LEASE_TTL_SECS,
            )
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        if !marked {
            let _ = self
                .store
                .update_job_status(
                    ingest_job_id,
                    LibraryIngestStatus::Failed,
                    None,
                    Some(LibraryIngestFailureStage::Other),
                    Some("parent URL import job is no longer active"),
                    false,
                    true,
                )
                .await;
            return Ok(());
        }
        match self.run_retry_ingest(file_id, ingest_job_id, kind).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let stage = self
                    .store
                    .get_job(ingest_job_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|job| job.failure_stage)
                    .unwrap_or(LibraryIngestFailureStage::Other);
                Err(IngestFailure::new(stage, error))
            }
        }
    }

    async fn await_ingest(&self, job_id: Uuid) -> IngestResult<()> {
        loop {
            let job = self
                .store
                .get_job(job_id)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
                .context("unknown ingest job")
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
            match job.status {
                LibraryIngestStatus::Succeeded => return Ok(()),
                LibraryIngestStatus::Failed => {
                    return Err(IngestFailure::new(
                        job.failure_stage
                            .unwrap_or(LibraryIngestFailureStage::Other),
                        anyhow!(job.error_message.unwrap_or_else(|| "ingest_failed".into())),
                    ));
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
            }
        }
    }

    async fn url_import_response(
        &self,
        job: UrlImportJobRecord,
    ) -> Result<LibraryUrlImportJobResponse> {
        let file = match job.file_id {
            Some(id) => self
                .store
                .get_file_in_project(job.group_id, id)
                .await?
                .map(|file| file_to_summary(&file)),
            None => None,
        };
        let ingest_job = match job.ingest_job_id {
            Some(id) => self
                .store
                .get_job_in_project(job.group_id, id)
                .await?
                .map(job_to_response),
            None => None,
        };
        Ok(LibraryUrlImportJobResponse {
            import_job_id: job.id,
            group_path: self.store.url_import_group_path(job.group_id).await?,
            source_url: job.source_url,
            status: parse_status(&job.status)?,
            attempt_count: job.attempt_count,
            file,
            ingest_job,
            error_code: job.error_code,
            error_message: job.error_message,
            failure_stage: job.failure_stage.as_deref().map(str::parse).transpose()?,
            created_at: job.created_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            updated_at: job.updated_at,
        })
    }
}

fn job_metadata(job: &UrlImportJobRecord) -> LibraryFileUploadMetadata {
    LibraryFileUploadMetadata {
        external_id: job.external_id.clone(),
        source_uri: job.source_uri.clone(),
        published_at: job.published_at,
        metadata_json: job.metadata_json.clone(),
    }
}

fn job_translation(job: &UrlImportJobRecord) -> Option<crate::contracts::TranslationDirective> {
    job.translation_provided
        .then(|| crate::contracts::TranslationDirective {
            source_locale: job.translation_source_locale.clone(),
            target_locales: job.translation_target_locales.clone(),
        })
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn hex_digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_status(value: &str) -> Result<LibraryUrlImportStatus> {
    match value {
        "queued" => Ok(LibraryUrlImportStatus::Queued),
        "downloading" => Ok(LibraryUrlImportStatus::Downloading),
        "ingesting" => Ok(LibraryUrlImportStatus::Ingesting),
        "succeeded" => Ok(LibraryUrlImportStatus::Succeeded),
        "failed" => Ok(LibraryUrlImportStatus::Failed),
        _ => Err(anyhow!("invalid URL import status")),
    }
}
