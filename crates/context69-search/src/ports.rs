use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts::{DocumentResponse, SearchHit, SearchRequest};
use uuid::Uuid;

use crate::{AccessScope, SearchPointHit, SearchSettings, StoredRerankItemScore};

#[async_trait]
pub trait SearchScopeResolver: Send + Sync {
    async fn access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope>;
}

#[async_trait]
pub trait SearchRepository: Send + Sync {
    async fn get_search_settings(&self) -> Result<Option<SearchSettings>>;
    async fn get_search_generation(&self) -> Result<i64>;
    async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        scope: &AccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>>;
    async fn keyword_search(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
        limit: usize,
    ) -> Result<Vec<SearchHit>>;
    async fn list_rerank_item_scores(
        &self,
        rerank_model: &str,
        query_hash: &str,
        chunk_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, StoredRerankItemScore>>;
    async fn upsert_rerank_item_scores(&self, scores: &[StoredRerankItemScore]) -> Result<()>;
    async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<Option<DocumentResponse>>;
}

#[async_trait]
pub trait SearchEmbeddingProvider: Send + Sync {
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>>;
}

#[async_trait]
pub trait SearchIndex: Send + Sync {
    async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<Vec<SearchPointHit>>;
}
