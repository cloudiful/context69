use context69_contracts::{LibraryFileDetailResponse, LibraryUploadResponse, MoveFileRequest};
use reqwest::{Method, multipart::Part};
use uuid::Uuid;

use super::Context69Client;
use crate::{Error, client::transport::file_upload_form};

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
