use std::sync::Arc;

use anyhow::{Context, Result};

use crate::contracts::DocumentResponse;
use crate::db::Database;
use crate::domain::AccessScope;
use crate::embedding::EmbeddingProvider;
use crate::qdrant_index::QdrantIndex;
use crate::rerank::OpenRouterRerankClient;
use crate::search_cache::SearchCache;
use crate::services::auth::AuthService;

mod cache_merge;
mod ranking;
mod search;

#[derive(Clone)]
pub struct QueryService {
    db: Database,
    embedding: Arc<dyn EmbeddingProvider>,
    index: QdrantIndex,
    rerank: OpenRouterRerankClient,
    cache: SearchCache,
    embedding_model: String,
    auth: AuthService,
}

impl QueryService {
    pub fn new(
        db: Database,
        embedding: Arc<dyn EmbeddingProvider>,
        index: QdrantIndex,
        cache: SearchCache,
        embedding_model: String,
        auth: AuthService,
    ) -> Result<Self> {
        Ok(Self {
            db,
            embedding,
            index,
            rerank: OpenRouterRerankClient::new()?,
            cache,
            embedding_model,
            auth,
        })
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        self.db
            .get_document(document_id, scope)
            .await?
            .context("document not found")
    }
}
