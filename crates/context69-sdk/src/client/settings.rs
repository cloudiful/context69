use context69_contracts::{
    DoclingSettingsResponse, ProviderAccountResponse, RuntimeSettingsResponse,
    SearchSettingsResponse, UpdateDoclingSettingsRequest, UpdateRuntimeSettingsRequest,
    UpdateSearchSettingsRequest, UpsertProviderAccountRequest,
};
use reqwest::Method;

use crate::{Context69Client, Error};

pub struct SettingsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SettingsApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/runtime")
                    .await?,
            )
            .await
    }

    pub async fn update_runtime_settings(
        &self,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<RuntimeSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, "/v1/settings/runtime")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn list_provider_accounts(&self) -> Result<Vec<ProviderAccountResponse>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/provider-accounts")
                    .await?,
            )
            .await
    }

    pub async fn create_provider_account(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/settings/provider-accounts")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn update_provider_account(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, "/v1/settings/provider-accounts")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn delete_provider_account(&self, account_key: &str) -> Result<(), Error> {
        let path = format!("/v1/settings/provider-accounts/{account_key}");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }

    pub async fn get_docling_settings(&self) -> Result<DoclingSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/docling")
                    .await?,
            )
            .await
    }

    pub async fn update_docling_settings(
        &self,
        request: &UpdateDoclingSettingsRequest,
    ) -> Result<DoclingSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, "/v1/settings/docling")
                    .await?
                    .json(request),
            )
            .await
    }

    pub async fn get_search_settings(&self) -> Result<SearchSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/search")
                    .await?,
            )
            .await
    }

    pub async fn update_search_settings(
        &self,
        request: &UpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, "/v1/settings/search")
                    .await?
                    .json(request),
            )
            .await
    }
}
