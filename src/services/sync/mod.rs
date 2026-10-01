use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};

use crate::domain_errors::DomainError;
use context69_translation::{TranslationCoordinator, TranslationService};
use futures::{StreamExt, stream};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::{OwnedMutexGuard, RwLock};
use tracing::{error, info, warn};

use crate::{
    chunking::{ChunkingConfig, chunk_document},
    config::{SourceConfig, SyncStrategy},
    contracts::{
        SourceConfigInput, SourceConnectionResponse, SourceOriginStatusKind, SourcePageResponse,
        SourceStatus, SyncOutcome, UpsertSourceConnectionRequest, VectorIndexRebuildState,
        VectorIndexRebuildStatus,
    },
    db::{Database, StoredSourceConnection},
    domain::SyncCheckpoint,
    embedding::EmbeddingProvider,
    normalize::normalize_record,
    qdrant_index::QdrantIndex,
    services::secret_store::SecretStore,
    source_store::SourceStore,
    sources::SourceConnector,
};

use super::source_registry::SourceRegistry;

mod connections;
mod execution;
pub(crate) mod project_source_folders;
mod runtime;
mod sources;

use connections::SourceConnectionSecrets;

pub(crate) use connections::save_source_connection;

/// One source connection write once the caller's request has been folded onto
/// what is stored: the trimmed name and the database URL that will be in effect.
pub(super) struct PendingSourceConnection {
    pub(super) name: String,
    pub(super) database_url: String,
}

#[derive(Clone)]
struct SyncRuntime {
    embedding: Arc<dyn EmbeddingProvider>,
    index: QdrantIndex,
}

#[derive(Clone)]
pub struct SyncService {
    db: Database,
    store: SecretStore,
    runtime: Option<SyncRuntime>,
    chunking: ChunkingConfig,
    max_concurrency: usize,
    source_pools: Arc<RwLock<HashMap<String, PgPool>>>,
    registry: Arc<RwLock<SourceRegistry>>,
    source_store: SourceStore,
    source_connection_statuses: Arc<RwLock<HashMap<String, SourceConnectionHealth>>>,
    vector_rebuild_status: Arc<RwLock<VectorIndexRebuildStatus>>,
    translation: TranslationService,
}

#[derive(Clone, Debug)]
struct SourceConnectionHealth {
    has_database_url: bool,
    status: SourceOriginStatusKind,
    message: Option<String>,
}

impl SyncService {
    const REINDEX_BATCH_SIZE: usize = 64;

    pub fn new(
        db: Database,
        store: SecretStore,
        embedding: Option<Arc<dyn EmbeddingProvider>>,
        index: Option<QdrantIndex>,
        chunking: ChunkingConfig,
        max_concurrency: usize,
        translation: TranslationService,
    ) -> Self {
        let runtime = embedding
            .zip(index)
            .map(|(embedding, index)| SyncRuntime { embedding, index });
        Self {
            db: db.clone(),
            store,
            runtime,
            chunking,
            max_concurrency,
            source_pools: Arc::new(RwLock::new(HashMap::new())),
            registry: Arc::new(RwLock::new(
                SourceRegistry::new(Vec::new(), &HashMap::new(), &HashMap::new())
                    .expect("empty source registry to initialize"),
            )),
            source_store: SourceStore::new(db),
            source_connection_statuses: Arc::new(RwLock::new(HashMap::new())),
            vector_rebuild_status: Arc::new(RwLock::new(VectorIndexRebuildStatus {
                state: VectorIndexRebuildState::Idle,
                processed_chunks: 0,
                total_chunks: 0,
                error_message: None,
                started_at: None,
                finished_at: None,
            })),
            translation,
        }
    }

    fn runtime(&self) -> Result<&SyncRuntime> {
        self.runtime.as_ref().ok_or_else(sync_runtime_unavailable)
    }

    /// The store binding for the source connections' sealed database URLs.
    fn source_secrets(&self) -> SourceConnectionSecrets {
        SourceConnectionSecrets::new(self.store.clone())
    }

    pub fn runtime_configured(&self) -> bool {
        self.runtime.is_some()
    }

    pub async fn seed_sources_if_empty(&self, sources: &[SourceConfig]) -> Result<()> {
        self.source_store.seed_sources_if_empty(sources).await
    }

    async fn acquire_lock(&self, source_key: &str) -> Result<OwnedMutexGuard<()>> {
        let lock = self
            .registry
            .read()
            .await
            .lock(source_key)
            .ok_or_else(|| DomainError::not_found(format!("unknown source {source_key}")))
            .map_err(anyhow::Error::from)?;
        Ok(lock.lock_owned().await)
    }

    async fn source_runtime(
        &self,
        source_key: &str,
    ) -> Result<Option<(SourceConfig, Arc<dyn SourceConnector>)>> {
        let registry = self.registry.read().await;
        let source = registry.config(source_key);
        let connector = registry.connector(source_key);

        Ok(match (source, connector) {
            (Some(source), Some(connector)) => Some((source, connector)),
            (None, None) => None,
            (Some(_), None) => {
                return Err(DomainError::unavailable(format!(
                    "source origin is unavailable for {source_key}"
                ))
                .into());
            }
            _ => {
                return Err(DomainError::internal(format!(
                    "incomplete source registry for {source_key}"
                ))
                .into());
            }
        })
    }

    async fn connection_names(&self) -> Result<Vec<String>> {
        // Names only: a validation or folder view never needs a database URL, so
        // it never opens one.
        Ok(self
            .db
            .list_source_connections()
            .await?
            .into_iter()
            .map(|connection| connection.name)
            .collect())
    }

    pub(crate) async fn connection_names_for_source_folders(&self) -> Result<Vec<String>> {
        self.connection_names().await
    }

    /// The connection a write will persist.
    ///
    /// A submitted database URL wins. Otherwise the one already in effect is
    /// resolved through the store, which fails closed rather than falling back to
    /// a stale column, and a connection that does not exist yet has nothing to
    /// keep — so it requires one.
    async fn resolve_source_connection(
        &self,
        connection_name: &str,
        database_url: Option<String>,
    ) -> Result<PendingSourceConnection> {
        let name = connection_name.trim();
        if name.is_empty() {
            return Err(
                DomainError::invalid_argument("source connection name must not be empty").into(),
            );
        }

        let existing = self.db.get_source_connection(name).await?;
        let database_url = match database_url.as_deref().map(str::trim) {
            Some(requested) if !requested.is_empty() => requested.to_string(),
            _ => match existing.as_ref() {
                Some(existing) => self
                    .source_secrets()
                    .resolve(existing)
                    .await?
                    .ok_or_else(missing_source_connection_database_url)?,
                None => return Err(missing_source_connection_database_url().into()),
            },
        };

        Ok(PendingSourceConnection {
            name: name.to_string(),
            database_url,
        })
    }

    /// Seals the database URL and saves the row, legacy column included.
    async fn persist_source_connection(
        &self,
        pending: &PendingSourceConnection,
    ) -> Result<StoredSourceConnection> {
        connections::save_source_connection(
            &self.db,
            &self.store,
            &pending.name,
            &pending.database_url,
        )
        .await
    }

    pub(crate) async fn upsert_source_connection_for_source_folder(
        &self,
        input: &SourceConfigInput,
    ) -> Result<()> {
        self.upsert_source_connection_for_source(input).await
    }
}

fn sync_runtime_unavailable() -> anyhow::Error {
    DomainError::unavailable(
        "sync runtime is not configured; save runtime settings and restart the service",
    )
    .into()
}

fn missing_source_connection_database_url() -> DomainError {
    DomainError::invalid_argument(
        "source connection database_url is required when creating a new connection",
    )
}
