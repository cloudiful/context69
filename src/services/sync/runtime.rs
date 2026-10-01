use super::*;

use crate::domain_errors::DomainError;

impl SyncService {
    pub async fn reload_sources(&self) -> Result<()> {
        let source_connections = self.db.list_source_connections().await?;
        let source_configs = self.source_store.list_source_configs().await?;
        let (source_pools, source_connection_statuses) =
            self.build_source_pools(&source_connections).await;
        let existing_locks = self.registry.read().await.locks_snapshot();
        let registry = SourceRegistry::new(source_configs, &source_pools, &existing_locks)?;
        *self.source_pools.write().await = source_pools;
        *self.registry.write().await = registry;
        *self.source_connection_statuses.write().await = source_connection_statuses;
        Ok(())
    }

    /// Builds one pool per connection, plus the health each one starts with.
    ///
    /// A connection with no database URL is misconfigured, and one whose sealed
    /// URL this deployment cannot open is reported unreachable rather than
    /// silently falling back to the legacy column. Neither blocks startup, which
    /// is the same contract a failed connect already had.
    async fn build_source_pools(
        &self,
        connections: &[StoredSourceConnection],
    ) -> (
        HashMap<String, PgPool>,
        HashMap<String, SourceConnectionHealth>,
    ) {
        let secrets = self.source_secrets();
        let mut pools = HashMap::new();
        let mut statuses = HashMap::new();
        for connection in connections {
            let name = connection.name.clone();
            let database_url = match secrets.resolve(connection).await {
                Ok(Some(database_url)) => database_url,
                Ok(None) => {
                    statuses.insert(
                        name,
                        SourceConnectionHealth {
                            has_database_url: false,
                            status: SourceOriginStatusKind::Misconfigured,
                            message: Some("database_url is empty".to_string()),
                        },
                    );
                    continue;
                }
                Err(error) => {
                    warn!(connection = name, error = %error, "source connection credential is unavailable; continuing without this source pool");
                    statuses.insert(
                        name,
                        SourceConnectionHealth {
                            has_database_url: true,
                            status: SourceOriginStatusKind::Unreachable,
                            message: Some(error.to_string()),
                        },
                    );
                    continue;
                }
            };

            match PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(3))
                .connect(&database_url)
                .await
            {
                Ok(pool) => {
                    pools.insert(name.clone(), pool);
                    statuses.insert(
                        name,
                        SourceConnectionHealth {
                            has_database_url: true,
                            status: SourceOriginStatusKind::Connected,
                            message: None,
                        },
                    );
                }
                Err(error) => {
                    warn!(connection = name, error = %error, "failed to connect source pool; continuing without blocking startup");
                    statuses.insert(
                        name,
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

    pub async fn validate_sources(&self) -> Result<()> {
        for (source_key, connector) in self.registry.read().await.connectors() {
            connector.validate().await.map_err(|error| {
                DomainError::invalid_argument(format!(
                    "failed to validate source {source_key}: {error}"
                ))
            })?;
        }
        Ok(())
    }

    pub async fn search_smoke_test(&self) -> Result<u64> {
        self.runtime()?.index.count_points().await
    }
}
