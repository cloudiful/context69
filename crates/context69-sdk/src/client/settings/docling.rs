use context69_contracts::{DoclingSettingsResponse, UpdateDoclingSettingsRequest};
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct DoclingSettingsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> DoclingSettingsApi<'a> {
    pub(super) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn get(&self) -> Result<DoclingSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/docling")
                    .await?,
            )
            .await
    }

    pub async fn update(
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
}
