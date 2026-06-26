use std::sync::Arc;

use anyhow::{Result, anyhow};
use context69_search::SearchService;

use crate::contracts::{DocumentResponse, SearchRequest, SearchResponse};
use crate::db::Database;
use crate::domain::AccessScope;
use crate::embedding::EmbeddingProvider;
use crate::qdrant_index::QdrantIndex;
use crate::services::auth::AuthService;

mod adapters;

use self::adapters::{AuthScopeResolver, DbSearchRepository, EmbeddingAdapter, QdrantSearchIndex};

#[derive(Clone)]
pub struct QueryService {
    db: Database,
    inner: Option<SearchService>,
}

impl QueryService {
    pub fn disabled(db: Database) -> Self {
        Self { db, inner: None }
    }

    pub async fn new(
        db: Database,
        embedding: Arc<dyn EmbeddingProvider>,
        index: QdrantIndex,
        valkey_url: Option<&str>,
        embedding_model: String,
        auth: AuthService,
    ) -> Result<Self> {
        Ok(Self {
            db: db.clone(),
            inner: Some(
                SearchService::new(
                    Arc::new(DbSearchRepository::new(db)),
                    Arc::new(AuthScopeResolver::new(auth)),
                    Arc::new(EmbeddingAdapter::new(embedding)),
                    Arc::new(QdrantSearchIndex::new(index)),
                    valkey_url,
                    embedding_model,
                )
                .await?,
            ),
        })
    }

    pub async fn search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        let inner = self.inner.as_ref().ok_or_else(search_runtime_unavailable)?;
        inner.search(user_id, request).await
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        self.db
            .get_document(document_id, scope)
            .await
            .map(|document| document.ok_or_else(|| anyhow!("document not found")))?
    }
}

fn search_runtime_unavailable() -> anyhow::Error {
    anyhow!(
        "search runtime is not configured; save runtime/provider settings and restart the service"
    )
}
