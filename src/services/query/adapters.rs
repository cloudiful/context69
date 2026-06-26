use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use context69_search::{
    AccessScope as SearchAccessScope, SearchEmbeddingProvider, SearchIndex, SearchPointHit,
    SearchRepository, SearchScopeResolver, SearchSettings,
    StoredRerankItemScore as SearchStoredRerankItemScore,
};
use uuid::Uuid;

use crate::{
    contracts::{DocumentResponse, SearchHit, SearchRequest},
    db::{Database, StoredRerankItemScore},
    domain::AccessScope,
    embedding::EmbeddingProvider,
    qdrant_index::QdrantIndex,
    services::auth::AuthService,
};

#[derive(Clone)]
pub(super) struct DbSearchRepository {
    db: Database,
}

impl DbSearchRepository {
    pub(super) fn new(db: Database) -> Self {
        Self { db }
    }
}

#[derive(Clone)]
pub(super) struct AuthScopeResolver {
    auth: AuthService,
}

impl AuthScopeResolver {
    pub(super) fn new(auth: AuthService) -> Self {
        Self { auth }
    }
}

#[derive(Clone)]
pub(super) struct EmbeddingAdapter {
    embedding: Arc<dyn EmbeddingProvider>,
}

impl EmbeddingAdapter {
    pub(super) fn new(embedding: Arc<dyn EmbeddingProvider>) -> Self {
        Self { embedding }
    }
}

#[derive(Clone)]
pub(super) struct QdrantSearchIndex {
    index: QdrantIndex,
}

impl QdrantSearchIndex {
    pub(super) fn new(index: QdrantIndex) -> Self {
        Self { index }
    }
}

pub(super) fn to_search_scope(scope: &AccessScope) -> SearchAccessScope {
    SearchAccessScope {
        user_id: scope.user_id,
        include_public: scope.include_public,
        private_project_ids: scope.private_project_ids.clone(),
        group_key: scope.group_key.clone(),
        project_key: scope.project_key.clone(),
    }
}

fn to_root_scope(scope: &SearchAccessScope) -> AccessScope {
    AccessScope {
        user_id: scope.user_id,
        include_public: scope.include_public,
        private_project_ids: scope.private_project_ids.clone(),
        group_key: scope.group_key.clone(),
        project_key: scope.project_key.clone(),
    }
}

fn to_search_settings(settings: crate::db::StoredSearchSettings) -> SearchSettings {
    SearchSettings {
        mode: settings.mode,
        rerank_enabled: settings.rerank_enabled,
        rerank_base_url: settings.rerank_base_url,
        rerank_model: settings.rerank_model,
        candidate_limit: settings.candidate_limit,
        timeout_secs: settings.timeout_secs,
        api_key: settings.api_key,
    }
}

#[async_trait]
impl SearchRepository for DbSearchRepository {
    async fn get_search_settings(&self) -> Result<Option<SearchSettings>> {
        Ok(self.db.get_search_settings().await?.map(to_search_settings))
    }

    async fn get_search_generation(&self) -> Result<i64> {
        self.db.get_search_generation().await
    }

    async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        scope: &SearchAccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>> {
        self.db
            .fetch_search_hits_by_chunk_ids(chunk_ids, &to_root_scope(scope))
            .await
    }

    async fn keyword_search(
        &self,
        request: &SearchRequest,
        scope: &SearchAccessScope,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        self.db
            .keyword_search(request, &to_root_scope(scope), limit)
            .await
    }

    async fn list_rerank_item_scores(
        &self,
        rerank_model: &str,
        query_hash: &str,
        chunk_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, SearchStoredRerankItemScore>> {
        Ok(self
            .db
            .list_rerank_item_scores(rerank_model, query_hash, chunk_ids)
            .await?
            .into_iter()
            .map(|(chunk_id, score)| {
                (
                    chunk_id,
                    SearchStoredRerankItemScore {
                        rerank_model: score.rerank_model,
                        query_hash: score.query_hash,
                        query_text_trimmed: score.query_text_trimmed,
                        chunk_id: score.chunk_id,
                        score: score.score,
                    },
                )
            })
            .collect())
    }

    async fn upsert_rerank_item_scores(
        &self,
        scores: &[SearchStoredRerankItemScore],
    ) -> Result<()> {
        let stored = scores
            .iter()
            .map(|score| StoredRerankItemScore {
                rerank_model: score.rerank_model.clone(),
                query_hash: score.query_hash.clone(),
                query_text_trimmed: score.query_text_trimmed.clone(),
                chunk_id: score.chunk_id,
                score: score.score,
            })
            .collect::<Vec<_>>();
        self.db.upsert_rerank_item_scores(&stored).await
    }

    async fn get_document(
        &self,
        document_id: i64,
        scope: &SearchAccessScope,
    ) -> Result<Option<DocumentResponse>> {
        self.db
            .get_document(document_id, &to_root_scope(scope))
            .await
    }
}

#[async_trait]
impl SearchScopeResolver for AuthScopeResolver {
    async fn access_scope(
        &self,
        user_id: Option<i64>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<SearchAccessScope> {
        let scope = self
            .auth
            .access_scope(user_id, group_key, project_key)
            .await?;
        Ok(to_search_scope(&scope))
    }
}

#[async_trait]
impl SearchEmbeddingProvider for EmbeddingAdapter {
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        self.embedding.embed_query(query).await
    }
}

#[async_trait]
impl SearchIndex for QdrantSearchIndex {
    async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &SearchAccessScope,
    ) -> Result<Vec<SearchPointHit>> {
        Ok(self
            .index
            .search(vector, request, &to_root_scope(scope))
            .await?
            .into_iter()
            .map(|hit| SearchPointHit {
                chunk_id: hit.chunk_id,
                score: hit.score,
            })
            .collect())
    }
}
