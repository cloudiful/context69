use context69_contracts::{ProviderAccountResponse, UpsertProviderAccountRequest};
use reqwest::Method;

use super::Context69Client;
use crate::{Error, client::transport::encode_path_component};

pub struct ProviderAccountsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> ProviderAccountsApi<'a> {
    pub(super) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<Vec<ProviderAccountResponse>, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/provider-accounts")
                    .await?,
            )
            .await
    }

    pub async fn create(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse, Error> {
        self.save(Method::POST, request).await
    }

    pub async fn update(
        &self,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse, Error> {
        self.save(Method::PUT, request).await
    }

    async fn save(
        &self,
        method: Method,
        request: &UpsertProviderAccountRequest,
    ) -> Result<ProviderAccountResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(method, "/v1/settings/provider-accounts")
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct ProviderAccountApi<'a> {
    client: &'a Context69Client,
    account_key: String,
}

impl<'a> ProviderAccountApi<'a> {
    pub(super) fn new(client: &'a Context69Client, account_key: String) -> Self {
        Self {
            client,
            account_key,
        }
    }

    pub async fn delete(&self) -> Result<(), Error> {
        let path = format!(
            "/v1/settings/provider-accounts/{}",
            encode_path_component(&self.account_key)
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
