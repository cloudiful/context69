use context69_contracts::{
    CreateSourceFolderRequest, SourceConfigInput, SourceFolderResponse, TaskRef,
};
use reqwest::Method;
use uuid::Uuid;

use super::Context69Client;
use crate::{Error, client::transport::group_path};

pub struct GroupSourceFoldersApi<'a> {
    client: &'a Context69Client,
    group_path: String,
}

impl<'a> GroupSourceFoldersApi<'a> {
    pub(super) fn new(client: &'a Context69Client, group_path: String) -> Self {
        Self { client, group_path }
    }

    pub async fn create(
        &self,
        request: &CreateSourceFolderRequest,
    ) -> Result<SourceFolderResponse, Error> {
        let path = group_path(&self.group_path, "/source-folders");
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

pub struct GroupSourceFolderApi<'a> {
    client: &'a Context69Client,
    group_path: String,
    folder_id: Uuid,
}

impl<'a> GroupSourceFolderApi<'a> {
    pub(super) fn new(client: &'a Context69Client, group_path: String, folder_id: Uuid) -> Self {
        Self {
            client,
            group_path,
            folder_id,
        }
    }

    pub async fn update(&self, request: &SourceConfigInput) -> Result<SourceFolderResponse, Error> {
        let suffix = format!("/source-folders/{}/config", self.folder_id);
        let path = group_path(&self.group_path, &suffix);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn sync(&self) -> Result<TaskRef, Error> {
        let suffix = format!("/source-folders/{}/sync", self.folder_id);
        let path = group_path(&self.group_path, &suffix);
        self.client
            .execute_json(self.client.authorized_request(Method::POST, &path).await?)
            .await
    }
}
