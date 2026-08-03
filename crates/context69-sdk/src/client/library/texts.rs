use context69_contracts::{CreateTextRequest, TaskRef, UpsertLibraryTextRequest};
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct LibraryTextsApi<'a> {
    client: &'a Context69Client,
    base_path: String,
}

impl<'a> LibraryTextsApi<'a> {
    pub(super) fn new(client: &'a Context69Client, base_path: String) -> Self {
        Self { client, base_path }
    }

    pub async fn create(
        &self,
        request: &CreateTextRequest,
    ) -> Result<TaskRef, Error> {
        create_text(self.client, &self.base_path, request).await
    }
}

pub struct GroupLibraryTextsApi<'a> {
    client: &'a Context69Client,
    base_path: String,
}

impl<'a> GroupLibraryTextsApi<'a> {
    pub(super) fn new(client: &'a Context69Client, base_path: String) -> Self {
        Self { client, base_path }
    }

    pub async fn create(
        &self,
        request: &CreateTextRequest,
    ) -> Result<TaskRef, Error> {
        create_text(self.client, &self.base_path, request).await
    }

    pub async fn upsert(
        &self,
        request: &UpsertLibraryTextRequest,
    ) -> Result<TaskRef, Error> {
        let path = format!("{}/texts", self.base_path);
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }
}

async fn create_text(
    client: &Context69Client,
    base_path: &str,
    request: &CreateTextRequest,
) -> Result<TaskRef, Error> {
    let path = format!("{base_path}/texts");
    client
        .execute_json(
            client
                .authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
}
