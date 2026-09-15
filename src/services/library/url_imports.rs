use super::*;
use crate::contracts::ImportLibraryFileFromUrlRequest;
use anyhow::Result;

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
                .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))
                .map_err(anyhow::Error::from)?;
        }

        let trusted_proxy_enabled = self.settings.trusted_proxy_enabled().await?;
        let limiter = self
            .url_import_runtime
            .limiter()
            .ok_or_else(|| DomainError::unavailable("URL import rate limiter is unavailable"))
            .map_err(anyhow::Error::from)?;
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
