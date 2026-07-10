use context69_contracts::{SearchSettingsResponse, UpdateSearchSettingsRequest};
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct SearchSettingsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SearchSettingsApi<'a> {
    pub(super) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn get(&self) -> Result<SearchSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/search")
                    .await?,
            )
            .await
    }

    pub async fn update(
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
