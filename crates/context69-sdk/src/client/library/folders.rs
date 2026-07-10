use context69_contracts::{CreateFolderRequest, LibraryFolderResponse, MoveFolderRequest};
use reqwest::Method;
use uuid::Uuid;

use super::Context69Client;
use crate::Error;

pub struct LibraryFoldersApi<'a> {
    client: &'a Context69Client,
    base_path: String,
}

impl<'a> LibraryFoldersApi<'a> {
    pub(super) fn new(client: &'a Context69Client, base_path: String) -> Self {
        Self { client, base_path }
    }

    pub async fn create(
        &self,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!("{}/folders", self.base_path);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct LibraryFolderApi<'a> {
    client: &'a Context69Client,
    base_path: String,
    folder_id: Uuid,
}

impl<'a> LibraryFolderApi<'a> {
    pub(super) fn new(client: &'a Context69Client, base_path: String, folder_id: Uuid) -> Self {
        Self {
            client,
            base_path,
            folder_id,
        }
    }

    pub async fn move_to(
        &self,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse, Error> {
        let path = format!("{}/folders/{}/move", self.base_path, self.folder_id);
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
        let path = format!("{}/folders/{}", self.base_path, self.folder_id);
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
