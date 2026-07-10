use context69_contracts::{
    SourceConfigInput, SourceConnectionResponse, SourceStatus, SyncOutcome,
    UpsertSourceConnectionRequest,
};
use reqwest::Method;

use super::Context69Client;
use crate::{Error, client::transport::encode_path_component};

pub struct SourcesApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SourcesApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<Vec<SourceStatus>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/sources")
                    .await?,
            )
            .await
    }

    pub async fn create(&self, request: &SourceConfigInput) -> Result<SourceStatus, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/sources")
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct SourceApi<'a> {
    client: &'a Context69Client,
    source_key: String,
}

impl<'a> SourceApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, source_key: String) -> Self {
        Self { client, source_key }
    }

    fn path(&self, suffix: &str) -> String {
        format!(
            "/v1/sources/{}{}",
            encode_path_component(&self.source_key),
            suffix
        )
    }

    pub async fn update(&self, request: &SourceConfigInput) -> Result<SourceStatus, Error> {
        let path = self.path("");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn sync(&self) -> Result<SyncOutcome, Error> {
        let path = self.path("/sync");
        self.client
            .execute_json(self.client.authorized_request(Method::POST, &path).await?)
            .await
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let path = self.path("");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}

pub struct SourceConnectionsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SourceConnectionsApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<Vec<SourceConnectionResponse>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/source-connections")
                    .await?,
            )
            .await
    }

    pub async fn create(
        &self,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.save(Method::POST, request).await
    }

    pub async fn update(
        &self,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.save(Method::PUT, request).await
    }

    async fn save(
        &self,
        method: Method,
        request: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(method, "/v1/source-connections")
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct SourceConnectionApi<'a> {
    client: &'a Context69Client,
    name: String,
}

impl<'a> SourceConnectionApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, name: String) -> Self {
        Self { client, name }
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let path = format!(
            "/v1/source-connections/{}",
            encode_path_component(&self.name)
        );
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
