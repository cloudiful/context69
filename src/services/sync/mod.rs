use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use futures::{StreamExt, stream};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::{OwnedMutexGuard, RwLock};
use tracing::{error, info, warn};

use crate::{
    chunking::{ChunkingConfig, chunk_document},
    config::{SourceConfig, SyncStrategy},
    contracts::{
        SourceConfigInput, SourceConnectionResponse, SourceOriginStatusKind, SourceStatus,
        SyncOutcome, UpsertSourceConnectionRequest,
    },
    db::{Database, StoredSourceConnection},
    domain::{ChunkPayload, SyncCheckpoint},
    embedding::EmbeddingProvider,
    normalize::normalize_record,
    qdrant_index::QdrantIndex,
    source_store::SourceStore,
    sources::SourceConnector,
};

use super::source_registry::SourceRegistry;

mod connections;
mod execution;
mod runtime;
mod sources;

#[derive(Clone)]
struct SyncRuntime {
    embedding: Arc<dyn EmbeddingProvider>,
    index: QdrantIndex,
}

#[derive(Clone)]
pub struct SyncService {
    db: Database,
    runtime: Option<SyncRuntime>,
    chunking: ChunkingConfig,
    max_concurrency: usize,
    source_pools: Arc<RwLock<HashMap<String, PgPool>>>,
    registry: Arc<RwLock<SourceRegistry>>,
    source_store: SourceStore,
    source_connection_statuses: Arc<RwLock<HashMap<String, SourceConnectionHealth>>>,
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
        embedding: Option<Arc<dyn EmbeddingProvider>>,
        index: Option<QdrantIndex>,
        chunking: ChunkingConfig,
        max_concurrency: usize,
    ) -> Self {
        let runtime = embedding
            .zip(index)
            .map(|(embedding, index)| SyncRuntime { embedding, index });
        Self {
            db: db.clone(),
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
        }
    }

    fn runtime(&self) -> Result<&SyncRuntime> {
        self.runtime.as_ref().ok_or_else(sync_runtime_unavailable)
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
            .with_context(|| format!("unknown source {source_key}"))?;
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
                return Err(anyhow!("source origin is unavailable for {source_key}"));
            }
            _ => return Err(anyhow!("incomplete source registry for {source_key}")),
        })
    }

    async fn connection_names(&self) -> Result<Vec<String>> {
        Ok(self
            .db
            .list_source_connections()
            .await?
            .into_iter()
            .map(|connection| connection.name)
            .collect())
    }

    async fn resolve_source_connection(
        &self,
        connection_name: &str,
        database_url: Option<String>,
    ) -> Result<StoredSourceConnection> {
        let name = connection_name.trim();
        if name.is_empty() {
            return Err(anyhow!("source connection name must not be empty"));
        }

        let existing = self.db.get_source_connection(name).await?;
        let database_url = if let Some(database_url) = database_url {
            let trimmed = database_url.trim();
            if trimmed.is_empty() {
                existing
                    .map(|connection| connection.database_url)
                    .ok_or_else(|| {
                        anyhow!(
                            "source connection database_url is required when creating a new connection"
                        )
                    })?
            } else {
                trimmed.to_string()
            }
        } else if let Some(existing) = existing {
            existing.database_url
        } else {
            return Err(anyhow!(
                "source connection database_url is required when creating a new connection"
            ));
        };

        Ok(StoredSourceConnection {
            name: name.to_string(),
            database_url,
        })
    }
}

fn sync_runtime_unavailable() -> anyhow::Error {
    anyhow!(
        "sync runtime is not configured; save runtime/provider settings and restart the service"
    )
}

async fn build_source_pools(
    connections: &[StoredSourceConnection],
) -> (
    HashMap<String, PgPool>,
    HashMap<String, SourceConnectionHealth>,
) {
    let mut pools = HashMap::new();
    let mut statuses = HashMap::new();
    for connection in connections {
        let database_url = connection.database_url.trim();
        if database_url.is_empty() {
            statuses.insert(
                connection.name.clone(),
                SourceConnectionHealth {
                    has_database_url: false,
                    status: SourceOriginStatusKind::Misconfigured,
                    message: Some("database_url is empty".to_string()),
                },
            );
            continue;
        }

        match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(3))
            .connect(database_url)
            .await
        {
            Ok(pool) => {
                pools.insert(connection.name.clone(), pool);
                statuses.insert(
                    connection.name.clone(),
                    SourceConnectionHealth {
                        has_database_url: true,
                        status: SourceOriginStatusKind::Connected,
                        message: None,
                    },
                );
            }
            Err(error) => {
                warn!(connection = connection.name, error = %error, "failed to connect source pool; continuing without blocking startup");
                statuses.insert(
                    connection.name.clone(),
                    SourceConnectionHealth {
                        has_database_url: true,
                        status: SourceOriginStatusKind::Unreachable,
                        message: Some(error.to_string()),
                    },
                );
            }
        }
    }
    (pools, statuses)
}
