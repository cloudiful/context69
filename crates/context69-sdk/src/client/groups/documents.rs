use context69_contracts::{
    BatchGetDocumentsRequest, BatchGetDocumentsResponse, CreateMetadataIndexRequest, DocumentKey,
    DocumentLookupQuery, DocumentQueryRequest, DocumentQueryResponse, DocumentResponse,
    MetadataIndexResponse, UpdateMetadataIndexRequest,
};
use reqwest::Method;
use uuid::Uuid;

use super::super::Context69Client;
use crate::{Error, client::transport::group_path};

pub struct GroupDocumentsApi<'a> {
    client: &'a Context69Client,
    group_path: String,
}
impl<'a> GroupDocumentsApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, group_path: String) -> Self {
        Self { client, group_path }
    }
    pub async fn query(
        &self,
        request: &DocumentQueryRequest,
    ) -> Result<DocumentQueryResponse, Error> {
        let path = group_path(&self.group_path, "/documents/query");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }
    pub async fn get(&self, key: &DocumentKey) -> Result<DocumentResponse, Error> {
        self.get_localized(key, None).await
    }

    pub async fn get_localized(
        &self,
        key: &DocumentKey,
        locale: Option<&str>,
    ) -> Result<DocumentResponse, Error> {
        let path = group_path(&self.group_path, "/documents/by-external-id");
        let query = DocumentLookupQuery {
            source_key: key.source_key.clone(),
            external_id: key.external_id.clone(),
            locale: locale.map(str::to_string),
        };
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, &path)
                    .await?
                    .query(&query),
            )
            .await
    }
    pub async fn batch_get(
        &self,
        request: &BatchGetDocumentsRequest,
    ) -> Result<BatchGetDocumentsResponse, Error> {
        let path = group_path(&self.group_path, "/documents/batch-get");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .json(request),
            )
            .await
    }
    pub async fn delete(&self, key: &DocumentKey) -> Result<(), Error> {
        let path = group_path(&self.group_path, "/documents/by-external-id");
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?
                    .query(key),
            )
            .await
    }
}

pub struct GroupMetadataIndexesApi<'a> {
    client: &'a Context69Client,
    group_path: String,
    source_key: String,
}
impl<'a> GroupMetadataIndexesApi<'a> {
    pub(crate) fn new(client: &'a Context69Client, group_path: String, source_key: String) -> Self {
        Self {
            client,
            group_path,
            source_key,
        }
    }
    pub async fn list(&self) -> Result<Vec<MetadataIndexResponse>, Error> {
        let path = group_path(&self.group_path, "/metadata-indexes");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::GET, &path)
                    .await?
                    .query(&[("source_key", &self.source_key)]),
            )
            .await
    }
    pub async fn create(
        &self,
        request: &CreateMetadataIndexRequest,
    ) -> Result<MetadataIndexResponse, Error> {
        let path = group_path(&self.group_path, "/metadata-indexes");
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::POST, &path)
                    .await?
                    .query(&[("source_key", &self.source_key)])
                    .json(request),
            )
            .await
    }
    pub async fn update(
        &self,
        index_id: Uuid,
        request: &UpdateMetadataIndexRequest,
    ) -> Result<MetadataIndexResponse, Error> {
        let path = group_path(&self.group_path, &format!("/metadata-indexes/{index_id}"));
        self.client
            .execute_json(
                self.client
                    .authorized_request(Method::PUT, &path)
                    .await?
                    .json(request),
            )
            .await
    }
    pub async fn retry(&self, index_id: Uuid) -> Result<MetadataIndexResponse, Error> {
        let path = group_path(
            &self.group_path,
            &format!("/metadata-indexes/{index_id}/retry"),
        );
        self.client
            .execute_json(self.client.authorized_request(Method::POST, &path).await?)
            .await
    }
    pub async fn delete(&self, index_id: Uuid) -> Result<(), Error> {
        let path = group_path(&self.group_path, &format!("/metadata-indexes/{index_id}"));
        self.client
            .execute_empty(
                self.client
                    .authorized_request(Method::DELETE, &path)
                    .await?,
            )
            .await
    }
}
