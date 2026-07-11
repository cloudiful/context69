use context69_contracts::{
    LibraryFileDetailResponse, LibraryUploadResponse, MoveFileRequest, PrepareLibraryUploadRequest,
    PrepareLibraryUploadResponse,
};
use reqwest::{Method, multipart::Part};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::Context69Client;
use crate::{
    Error,
    client::transport::{file_upload_form, file_upload_form_with_sha256},
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
    ) -> Result<LibraryUploadResponse, Error> {
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

    pub async fn upload_bytes_deduplicated(
        &self,
        folder_id: Option<Uuid>,
        filename: impl Into<String>,
        media_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<LibraryUploadResponse, Error> {
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
            })
            .await?;
        if !prepared.upload_required {
            return Ok(LibraryUploadResponse {
                files: prepared.file.into_iter().collect(),
                jobs: prepared.job.into_iter().collect(),
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
                    .multipart(file_upload_form_with_sha256(folder_id, sha256, part)),
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

    pub async fn delete(&self) -> Result<(), Error> {
        let path = format!("{}/files/{}", self.base_path, self.file_id);
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
