use context69_contracts::{TranslationSettingsResponse, UpdateTranslationSettingsRequest};
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct TranslationSettingsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> TranslationSettingsApi<'a> {
    pub(super) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn get(&self) -> Result<TranslationSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/translation")
                    .await?,
            )
            .await
    }

    pub async fn update(
        &self,
        request: &UpdateTranslationSettingsRequest,
    ) -> Result<TranslationSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, "/v1/settings/translation")
                    .await?
                    .json(request),
            )
            .await
    }
}
