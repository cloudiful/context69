use context69_contracts::{
    CreateSourceFolderRequest, SourceConfigInput, SourceConnectionResponse, SourceFolderResponse,
    SourceStatus, SyncOutcome, UpsertSourceConnectionRequest,
};
use reqwest::Method;
use uuid::Uuid;

use crate::{Context69Client, Error, client::encode_path_component};

pub struct SourcesApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SourcesApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/sources")
                    .await?,
            )
            .await
    }

    pub async fn create_source(&self, request: &SourceConfigInput) -> Result<SourceStatus, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/sources")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn update_source(
        &self,
        source_key: &str,
        request: &SourceConfigInput,
    ) -> Result<SourceStatus, Error> {
        let path = format!("/v1/sources/{source_key}");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<(), Error> {
        let path = format!("/v1/sources/{source_key}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn sync_source(&self, source_key: &str) -> Result<SyncOutcome, Error> {
        let path = format!("/v1/sources/{source_key}/sync");
        self.client
            .execute_json(self.client.authorized_request(Method::POST, &path).await?)
            .await
    }

    pub async fn list_source_connections(&self) -> Result<Vec<SourceConnectionResponse>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/source-connections")
                    .await?,
            )
            .await
    }

    pub async fn create_source_connection(
        &self,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/source-connections")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn update_source_connection(
        &self,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, "/v1/source-connections")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_source_connection(&self, name: &str) -> Result<(), Error> {
        let path = format!("/v1/source-connections/{name}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn create_group_source_folder(
        &self,
        group_path: &str,
        request: &CreateSourceFolderRequest,
    ) -> Result<SourceFolderResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/source-folders",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn update_group_source_folder_config(
        &self,
        group_path: &str,
        folder_id: Uuid,
        request: &SourceConfigInput,
    ) -> Result<SourceFolderResponse, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/source-folders/{folder_id}/config",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn sync_group_source_folder(
        &self,
        group_path: &str,
        folder_id: Uuid,
    ) -> Result<SyncOutcome, Error> {
        let path = format!(
            "/v1/groups/by-path/{}/source-folders/{folder_id}/sync",
            encode_path_component(group_path)
        );
        self.client
            .execute_json(self.client.authorized_request(Method::POST, &path).await?)
            .await
    }
}
