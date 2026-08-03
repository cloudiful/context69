use context69_contracts::{RuntimeSettingsResponse, TaskRef, UpdateRuntimeSettingsRequest};
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct RuntimeSettingsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> RuntimeSettingsApi<'a> {
    pub(super) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn get(&self) -> Result<RuntimeSettingsResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, "/v1/settings/runtime")
                    .await?,
            )
            .await
    }

    pub async fn update(
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

    pub async fn rebuild_vector_index(&self) -> Result<TaskRef, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(
                        Method::POST,
                        "/v1/settings/runtime/vector-index/rebuild",
                    )
                    .await?,
            )
            .await
    }
}
