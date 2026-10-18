use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;

use crate::domain_errors::DomainError;
use context69_contracts::search::SearchStreamEvent;
use context69_search::SearchService;

use crate::contracts::{DocumentResponse, SearchRequest, SearchResponse};
use crate::db::Database;
use crate::domain::AccessScope;
use crate::embedding::EmbeddingRuntime;
use crate::qdrant_index::QdrantIndex;
use crate::services::auth::AuthService;
use crate::services::secret_store::SecretStore;
use crate::services::settings::secrets::SettingsSecrets;
use crate::vector_runtime::VectorRuntime;

mod adapters;

use adapters::{AuthScopeResolver, DbSearchRepository, EmbeddingAdapter, QdrantSearchIndex};
pub(crate) use context69_search::{DateWindowPage, SearchDatePointHit};

/// The collaborators a search service is built from.
///
/// Grouped so the constructor stays a single value: adding a dependency is a
/// field here rather than another positional argument at a call site.
pub struct QueryDeps<'a> {
    /// The application database.
    pub db: Database,
    /// The embedding provider.
    pub embedding: EmbeddingRuntime,
    /// The vector index.
    pub index: QdrantIndex,
    /// Valkey URL for distributed throttling.
    pub valkey_url: Option<&'a str>,
    /// Embedding model name, recorded in per-item scores.
    pub embedding_model: String,
    /// Authorization, shared with the scope resolver and the repository.
    pub auth: AuthService,
    /// Resolves the rerank API key through the shared store.
    pub store: SecretStore,
    /// Whether the vector index may serve reads.
    pub vector_index_ready: Arc<AtomicBool>,
}

/// The shared-runtime variant of [`QueryDeps`]: the search path takes its
/// embedding/index pair from the process-wide handle per request, so a Qdrant
/// that becomes available after startup starts serving without a restart.
pub struct SharedQueryDeps<'a> {
    pub db: Database,
    pub vector: VectorRuntime,
    pub valkey_url: Option<&'a str>,
    pub embedding_model: String,
    pub auth: AuthService,
    pub store: SecretStore,
}

#[derive(Clone)]
pub struct QueryService {
    db: Database,
    inner: Option<SearchService>,
    vector_index_ready: Arc<AtomicBool>,
}

impl QueryService {
    pub fn disabled(db: Database) -> Self {
        Self {
            db,
            inner: None,
            vector_index_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Builds the search service from its collaborators.
    ///
    /// `store` resolves the rerank API key per read, in memory. The search
    /// service never reads the legacy column directly and never holds a key it
    /// could log or echo.
    pub async fn new(deps: QueryDeps<'_>) -> Result<Self> {
        let vector = VectorRuntime::from_legacy(deps.embedding, Some(deps.index), "");
        Self::with_vector_runtime(SharedQueryDeps {
            db: deps.db,
            vector,
            valkey_url: deps.valkey_url,
            embedding_model: deps.embedding_model,
            auth: deps.auth,
            store: deps.store,
        })
        .await
    }

    /// Builds the search service onto the process-wide shared vector runtime.
    pub async fn with_vector_runtime(deps: SharedQueryDeps<'_>) -> Result<Self> {
        let SharedQueryDeps {
            db,
            vector,
            valkey_url,
            embedding_model,
            auth,
            store,
        } = deps;
        let vector_index_ready = vector.readiness_flag();
        Ok(Self {
            db: db.clone(),
            inner: Some(
                SearchService::new(
                    Arc::new(DbSearchRepository::new(
                        db,
                        auth.clone(),
                        vector.clone(),
                        SettingsSecrets::search(store),
                    )),
                    Arc::new(AuthScopeResolver::new(auth)),
                    Arc::new(EmbeddingAdapter::new(vector.clone())),
                    Arc::new(QdrantSearchIndex::new(vector)),
                    valkey_url,
                    embedding_model,
                )
                .await?,
            ),
            vector_index_ready,
        })
    }

    pub async fn search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        if !self.vector_index_ready.load(Ordering::Acquire) {
            return Err(DomainError::unavailable(
                "vector index is rebuilding or unavailable; retry after the rebuild completes",
            )
            .into());
        }
        let inner = self.inner.as_ref().ok_or_else(search_runtime_unavailable)?;
        inner.search(user_id, request).await
    }

    pub async fn stream_search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
        tx: tokio::sync::mpsc::Sender<SearchStreamEvent>,
        abort: context69_search::AbortSignal,
    ) -> Result<()> {
        if !self.vector_index_ready.load(Ordering::Acquire) {
            return Err(DomainError::unavailable(
                "vector index is rebuilding or unavailable; retry after the rebuild completes",
            )
            .into());
        }
        let inner = self.inner.as_ref().ok_or_else(search_runtime_unavailable)?;
        inner.stream_search(user_id, request, tx, abort).await
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        self.db
            .get_document_localized(document_id, locale, scope)
            .await
            .map(|document| document.ok_or_else(|| DomainError::not_found("document not found")))?
            .map_err(anyhow::Error::from)
    }
}

fn search_runtime_unavailable() -> anyhow::Error {
    DomainError::unavailable(
        "search runtime is not configured; save runtime settings and restart the service",
    )
    .into()
}
