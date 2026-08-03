use super::*;
use crate::contracts::ImportLibraryFileFromUrlRequest;
use anyhow::{Context, Result, anyhow};

impl LibraryService {
    pub(crate) async fn download_url_for_task(
        &self,
        group_id: i64,
        request: &ImportLibraryFileFromUrlRequest,
    ) -> Result<DownloadedLibraryFile> {
        let url = remote_download::normalize_url(&request.url)?;
        if let Some(folder_id) = request.folder_id {
            self.store
                .get_folder_in_project(group_id, folder_id)
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
        let trusted_proxy_enabled = self.settings.trusted_proxy_enabled().await?;
        let limiter = self
            .url_import_runtime
            .limiter()
            .ok_or_else(|| anyhow!("URL import rate limiter is unavailable"))?;
        let downloaded = remote_download::download(
            url.as_str(),
            request.filename.as_deref(),
            request.media_type.as_deref(),
            self.max_upload_size_bytes,
            trusted_proxy_enabled,
            limiter.as_ref(),
        )
        .await?;
        let sha256 = storage::hash_bytes(&downloaded.bytes);
        Ok(DownloadedLibraryFile {
            source_url: downloaded.url.to_string(),
            filename: downloaded.filename,
            media_type: downloaded.media_type,
            bytes: downloaded.bytes,
            sha256,
        })
    }
}
