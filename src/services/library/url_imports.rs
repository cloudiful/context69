use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::*;
use crate::{
    contracts::{
        ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata, LibraryUrlImportJobResponse,
        LibraryUrlImportStatus,
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
            })
            .await?;
        self.spawn_url_import(job.id);
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
        let job = self
            .store
            .retry_url_import_job(group_id, job_id)
            .await?
            .context("URL import job is not failed and cannot be retried")?;
        self.spawn_url_import(job.id);
        self.url_import_response(job).await
    }

    pub async fn resume_url_imports(&self) -> Result<()> {
        self.store.reset_interrupted_url_imports().await?;
        for id in self.store.list_pending_url_import_ids().await? {
            self.spawn_url_import(id);
        }
        Ok(())
    }

    fn spawn_url_import(&self, job_id: Uuid) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run_url_import(job_id).await {
                warn!(%job_id, %error, "URL import failed");
            }
        });
    }

    async fn run_url_import(&self, job_id: Uuid) -> Result<()> {
        let Some(job) = self.store.claim_url_import_job(job_id).await? else {
            return Ok(());
        };
        let result = if let Some(file_id) = job.file_id {
            self.retry_url_import_ingest(&job, file_id).await
        } else {
            self.download_and_import(&job).await
        };
        match result {
            Ok(()) => {
                self.store
                    .finish_url_import_job(job_id, "succeeded", None, None)
                    .await
            }
            Err(error) => {
                let message = error.to_string();
                let code = message.split(':').next().unwrap_or("url_import_failed");
                self.store
                    .finish_url_import_job(job_id, "failed", Some(code), Some(&message))
                    .await?;
                Err(error)
            }
        }
    }

    async fn download_and_import(&self, job: &UrlImportJobRecord) -> Result<()> {
        let downloaded = {
            let _permit = self.ingest_semaphore.acquire().await?;
            remote_download::download(
                &job.source_url,
                job.requested_filename.as_deref(),
                job.requested_media_type.as_deref(),
                self.max_upload_size_bytes,
            )
            .await?
        };
        let sha = storage::hash_bytes(&downloaded.bytes);
        let existing = self
            .store
            .get_file_by_sha_in_project(job.group_id, &sha)
            .await?;
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
                },
            )
            .await?;
        self.store
            .mark_url_import_ingesting(job.id, file.file_id, Some(ingest_job.job_id))
            .await?;
        self.await_ingest(ingest_job.job_id).await
    }

    async fn retry_url_import_ingest(&self, job: &UrlImportJobRecord, file_id: Uuid) -> Result<()> {
        let file = self
            .store
            .get_file_in_project(job.group_id, file_id)
            .await?
            .context("URL import file no longer exists")?;
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)?;
        let ingest_job_id = Uuid::new_v4();
        self.store.create_job(ingest_job_id, file_id).await?;
        self.store
            .mark_url_import_ingesting(job.id, file_id, Some(ingest_job_id))
            .await?;
        self.run_retry_ingest(file_id, ingest_job_id, kind).await
    }

    async fn await_ingest(&self, job_id: Uuid) -> Result<()> {
        loop {
            let job = self
                .store
                .get_job(job_id)
                .await?
                .context("unknown ingest job")?;
            match job.status {
                LibraryIngestStatus::Succeeded => return Ok(()),
                LibraryIngestStatus::Failed => {
                    return Err(anyhow!(
                        job.error_message.unwrap_or_else(|| "ingest_failed".into())
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
