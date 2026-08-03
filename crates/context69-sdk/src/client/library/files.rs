use context69_contracts::{
    ImportLibraryFileFromUrlRequest, LibraryFileDetailResponse, LibraryFileIngestOptions,
    LibraryFileUploadMetadata, MoveFileRequest, PrepareLibraryUploadRequest,
    PrepareLibraryUploadResponse, TaskRef,
};
use reqwest::{Method, multipart::Part};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::Context69Client;
use crate::{
    Error,
    client::transport::{file_upload_form, file_upload_form_with_options},
};

pub struct LibraryFilesApi<'a> {
    client: &'a Context69Client,
    base_path: String,
}

impl<'a> LibraryFilesApi<'a> {
    pub(super) fn new(client: &'a Context69Client, base_path: String) -> Self {
        Self { client, base_path }
    }

    pub async fn upload(
        &self,
        folder_id: Option<Uuid>,
        files: Vec<Part>,
    ) -> Result<TaskRef, Error> {
        let path = format!("{}/files/upload", self.base_path);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .multipart(file_upload_form(folder_id, files)),
            )
            .await
    }

    pub async fn prepare_upload(
        &self,
        request: &PrepareLibraryUploadRequest,
    ) -> Result<PrepareLibraryUploadResponse, Error> {
        let path = format!("{}/files/prepare-upload", self.base_path);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn import_url(
        &self,
        request: &ImportLibraryFileFromUrlRequest,
    ) -> Result<TaskRef, Error> {
        let path = format!("{}/files/import-url", self.base_path);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn upload_bytes_deduplicated(
        &self,
        folder_id: Option<Uuid>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<TaskRef, Error> {
        self.upload_bytes_deduplicated_with_metadata(folder_id, filename, media_type, bytes, None)
            .await
    }

    pub async fn upload_bytes_deduplicated_with_metadata(
        &self,
        folder_id: Option<Uuid>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        bytes: Vec<u8>,
        metadata: Option<LibraryFileUploadMetadata>,
    ) -> Result<TaskRef, Error> {
        let options = metadata.map(|metadata| LibraryFileIngestOptions {
            metadata,
            translation: None,
        });
        self.upload_bytes_deduplicated_with_options(folder_id, filename, media_type, bytes, options)
            .await
    }

    pub async fn upload_bytes_deduplicated_with_options(
        &self,
        folder_id: Option<Uuid>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        bytes: Vec<u8>,
        options: Option<LibraryFileIngestOptions>,
    ) -> Result<TaskRef, Error> {
        let filename = filename.into();
        let media_type = media_type.into();
        let sha256: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let prepared = self
            .prepare_upload(&PrepareLibraryUploadRequest {
                folder_id,
                filename: filename.clone(),
                media_type: media_type.clone(),
                size_bytes: bytes.len() as i64,
                sha256: sha256.clone(),
                metadata: options.as_ref().map(|value| value.metadata.clone()),
                translation: options.as_ref().and_then(|value| value.translation.clone()),
            })
            .await?;
        if !prepared.upload_required {
            return prepared.task.ok_or_else(|| {
                Error::InvalidResponse("prepared upload did not return a task".to_string())
            });
        }

        let part = Part::bytes(bytes)
            .file_name(filename)
            .mime_str(&media_type)?;
        let path = format!("{}/files/upload", self.base_path);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .multipart(file_upload_form_with_options(
                        folder_id,
                        sha256,
                        options.as_ref(),
                        part,
                    )?),
            )
            .await
    }
}

pub struct LibraryFileApi<'a> {
    client: &'a Context69Client,
    base_path: String,
    file_id: Uuid,
}

impl<'a> LibraryFileApi<'a> {
    pub(super) fn new(client: &'a Context69Client, base_path: String, file_id: Uuid) -> Self {
        Self {
            client,
            base_path,
            file_id,
        }
    }

    pub async fn get(&self) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!("{}/files/{}", self.base_path, self.file_id);
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn move_to(
        &self,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = format!("{}/files/{}/move", self.base_path, self.file_id);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete(&self) -> Result<TaskRef, Error> {
        let path = format!("{}/files/{}", self.base_path, self.file_id);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
