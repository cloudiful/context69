use super::*;

impl SyncService {
    pub async fn reload_sources(&self) -> Result<()> {
        let source_connections = self.db.list_source_connections().await?;
        let source_configs = self.source_store.list_source_configs().await?;
        let (source_pools, source_connection_statuses) =
            build_source_pools(&source_connections).await;
        let existing_locks = self.registry.read().await.locks_snapshot();
        let registry = SourceRegistry::new(source_configs, &source_pools, &existing_locks)?;
        *self.source_pools.write().await = source_pools;
        *self.registry.write().await = registry;
        *self.source_connection_statuses.write().await = source_connection_statuses;
        Ok(())
    }

    pub async fn validate_sources(&self) -> Result<()> {
        for (source_key, connector) in self.registry.read().await.connectors() {
            connector
                .validate()
                .await
                .with_context(|| format!("failed to validate source {source_key}"))?;
        }
        Ok(())
    }

    pub async fn search_smoke_test(&self) -> Result<u64> {
        self.runtime()?.index.count_points().await
    }
}
