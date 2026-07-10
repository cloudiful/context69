use context69_contracts::{DocumentResponse, SearchRequest, SearchResponse};
use reqwest::Method;

use super::Context69Client;
use crate::Error;

pub struct SearchApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SearchApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub async fn execute(&self, request: &SearchRequest) -> Result<SearchResponse, Error> {
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, "/v1/search")
                    .await?
                    .json(request),
            )
            .await
    }
}

pub struct DocumentApi<'a> {
    client: &'a Context69Client,
    document_id: i64,
}

impl<'a> DocumentApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, document_id: i64) -> Self {
        Self {
            client,
            document_id,
        }
    }

    pub async fn get(&self) -> Result<DocumentResponse, Error> {
        let path = format!("/v1/documents/{}", self.document_id);
        self.client
            .execute_json(self.client.authorized_request(Method::GET, &path).await?)
            .await
    }
}
