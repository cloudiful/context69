use context69_contracts::{DocumentResponse, SearchRequest, SearchResponse};
use reqwest::Method;

use crate::{Context69Client, Error};

pub struct SearchApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SearchApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn search(&self, request: SearchRequest) -> Result<SearchResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/search")
                    .await?
                    .json(&request),
            )
            .await
    }

    pub async fn get_document(&self, document_id: i64) -> Result<DocumentResponse, Error> {
        let path = format!("/v1/documents/{document_id}");
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }
}
