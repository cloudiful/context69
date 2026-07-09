use context69_contracts::{DocumentResponse, SearchRequest, SearchResponse};
use reqwest::Method;

use crate::{Context69Client, Error};

impl Context69Client {
    pub async fn search(&self, request: SearchRequest) -> Result<SearchResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/search")
                .await?
                .json(&request),
        )
        .await
    }

    pub async fn get_document(&self, document_id: i64) -> Result<DocumentResponse, Error> {
        let path = format!("/v1/documents/{document_id}");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }
}
