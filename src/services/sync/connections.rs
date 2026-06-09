use super::*;

impl SyncService {
    pub async fn list_source_connections(&self) -> Result<Vec<SourceConnectionResponse>> {
        let statuses = self.source_connection_statuses.read().await.clone();
        Ok(self
            .db
            .list_source_connections()
            .await?
            .into_iter()
            .map(|connection| SourceConnectionResponse {
                name: connection.name.clone(),
                has_database_url: !connection.database_url.trim().is_empty(),
                origin_status: statuses
                    .get(&connection.name)
                    .map(|status| status.status.clone())
                    .unwrap_or(SourceOriginStatusKind::Unknown),
                origin_message: statuses
                    .get(&connection.name)
                    .and_then(|status| status.message.clone()),
            })
            .collect())
    }

    pub async fn upsert_source_connection(
        &self,
        input: &UpsertSourceConnectionRequest,
    ) -> Result<SourceConnectionResponse> {
        let stored = self
            .resolve_source_connection(&input.name, input.database_url.clone())
            .await?;
        let saved = self.db.save_source_connection(&stored).await?;
        self.reload_sources().await?;
        Ok(SourceConnectionResponse {
            name: saved.name,
            has_database_url: !saved.database_url.trim().is_empty(),
            origin_status: SourceOriginStatusKind::Unknown,
            origin_message: None,
        })
    }

    pub async fn delete_source_connection(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("source connection name must not be empty"));
        }

        for source in self.source_store.list_source_configs().await? {
            if source.connection == name {
                return Err(anyhow!(
                    "source connection {name} is referenced by source {}",
                    source.key
                ));
            }
        }

        let deleted = self.db.delete_source_connection(name).await?;
        if !deleted {
            return Err(anyhow!("unknown source connection {name}"));
        }
        self.reload_sources().await?;
        Ok(())
    }
}
