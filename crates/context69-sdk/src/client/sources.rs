use context69_contracts::{
    SourceConfigInput, SourceConnectionResponse, SourceStatus, SyncOutcome,
    UpsertSourceConnectionRequest,
};
use reqwest::Method;

use crate::{Context69Client, Error};

impl Context69Client {
    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>, Error> {
        self.execute_json(self.authorized_request(Method::GET, "/v1/sources").await?)
            .await
    }

    pub async fn create_source(&self, request: &SourceConfigInput) -> Result<SourceStatus, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/sources")
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
        self.execute_json(
            self.authorized_request(Method::PUT, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<(), Error> {
        let path = format!("/v1/sources/{source_key}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    pub async fn sync_source(&self, source_key: &str) -> Result<SyncOutcome, Error> {
        let path = format!("/v1/sources/{source_key}/sync");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    pub async fn list_source_connections(&self) -> Result<Vec<SourceConnectionResponse>, Error> {
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/source-connections")
                .await?,
        )
        .await
    }

    pub async fn create_source_connection(
        &self,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/source-connections")
                .await?
                .json(request),
        )
        .await
    }

    pub async fn update_source_connection(
        &self,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::PUT, "/v1/source-connections")
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_source_connection(&self, name: &str) -> Result<(), Error> {
        let path = format!("/v1/source-connections/{name}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    pub async fn list_project_sources(
        &self,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<SourceStatus>, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/sources");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn create_project_source(
        &self,
        group_key: &str,
        project_key: &str,
        request: &SourceConfigInput,
    ) -> Result<SourceStatus, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/sources");
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn update_project_source(
        &self,
        group_key: &str,
        project_key: &str,
        source_key: &str,
        request: &SourceConfigInput,
    ) -> Result<SourceStatus, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}");
        self.execute_json(
            self.authorized_request(Method::PUT, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn delete_project_source(
        &self,
        group_key: &str,
        project_key: &str,
        source_key: &str,
    ) -> Result<(), Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    pub async fn sync_project_source(
        &self,
        group_key: &str,
        project_key: &str,
        source_key: &str,
    ) -> Result<SyncOutcome, Error> {
        let path =
            format!("/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}/sync");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }
}
