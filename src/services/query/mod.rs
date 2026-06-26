use std::sync::Arc;

use anyhow::Result;
use context69_search::SearchService;

use crate::contracts::{DocumentResponse, SearchRequest, SearchResponse};
use crate::db::Database;
use crate::domain::AccessScope;
use crate::embedding::EmbeddingProvider;
use crate::qdrant_index::QdrantIndex;
use crate::services::auth::AuthService;

mod adapters;

use self::adapters::{
    AuthScopeResolver, DbSearchRepository, EmbeddingAdapter, QdrantSearchIndex, to_search_scope,
};

#[derive(Clone)]
pub struct QueryService {
    inner: SearchService,
}

impl QueryService {
    pub async fn new(
        db: Database,
        embedding: Arc<dyn EmbeddingProvider>,
        index: QdrantIndex,
        valkey_url: Option<&str>,
        embedding_model: String,
        auth: AuthService,
    ) -> Result<Self> {
        Ok(Self {
            inner: SearchService::new(
                Arc::new(DbSearchRepository::new(db)),
                Arc::new(AuthScopeResolver::new(auth)),
                Arc::new(EmbeddingAdapter::new(embedding)),
                Arc::new(QdrantSearchIndex::new(index)),
                valkey_url,
                embedding_model,
            )
            .await?,
        })
    }

    pub async fn search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        self.inner.search(user_id, request).await
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        self.inner
            .get_document(document_id, &to_search_scope(scope))
            .await
    }
}
