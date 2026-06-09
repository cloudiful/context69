use super::*;

impl SyncService {
    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>> {
        self.decorate_sources(self.source_store.list_sources().await?).await
    }

    pub async fn list_sources_for_project(&self, project_id: i64) -> Result<Vec<SourceStatus>> {
        self.decorate_sources(self.source_store.list_sources_for_project(project_id).await?)
            .await
    }

    pub async fn get_source_for_project(
        &self,
        project_id: i64,
        source_key: &str,
    ) -> Result<Option<SourceStatus>> {
        let Some(source) = self
            .source_store
            .get_source_in_project(project_id, source_key)
            .await?
        else {
            return Ok(None);
        };
        let mut sources = self.decorate_sources(vec![source]).await?;
        Ok(sources.pop())
    }

    pub async fn create_source_in_scope(
        &self,
        scope: &crate::source_store::SourceScope,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        self.upsert_source_connection_for_source(input).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.insert_source_in_scope(&source, scope).await?;
        self.reload_sources().await?;
        self.get_source_for_project(scope.project_id, &source.key)
            .await?
            .with_context(|| format!("missing source {}", source.key))
    }

    pub async fn update_source_in_project(
        &self,
        project_id: i64,
        source_key: &str,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        if input.source_key != source_key {
            return Err(anyhow!("source_key cannot be changed"));
        }

        self.upsert_source_connection_for_source(input).await?;
        let _guard = self.acquire_lock(source_key).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store
            .update_source_in_project(Some(project_id), source_key, &source)
            .await?;
        self.reload_sources().await?;
        self.get_source_for_project(project_id, source_key)
            .await?
            .with_context(|| format!("missing source {source_key}"))
    }

    pub async fn delete_source_in_project(
        &self,
        project_id: i64,
        source_key: &str,
    ) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        let chunk_ids = self.source_store.list_source_chunk_ids(source_key).await?;
        self.index.delete_points(&chunk_ids).await?;
        let deleted = self
            .source_store
            .delete_source_in_project(Some(project_id), source_key)
            .await?;
        if !deleted {
            return Err(anyhow!("unknown source {source_key}"));
        }
        self.reload_sources().await?;
        Ok(())
    }

    async fn decorate_sources(&self, sources: Vec<SourceStatus>) -> Result<Vec<SourceStatus>> {
        let statuses = self.source_connection_statuses.read().await.clone();
        Ok(sources
            .into_iter()
            .map(|mut source| {
                if let Some(connection_status) = statuses.get(&source.connection) {
                    source.has_database_url = connection_status.has_database_url;
                    source.origin_status = connection_status.status.clone();
                    source.origin_message = connection_status.message.clone();
                }
                source
            })
            .collect())
    }

    pub async fn create_source(&self, input: &SourceConfigInput) -> Result<SourceStatus> {
        self.upsert_source_connection_for_source(input).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.insert_source(&source).await?;
        self.reload_sources().await?;
        self.source_store
            .get_source(&source.key)
            .await?
            .with_context(|| format!("missing source {}", source.key))
    }

    pub async fn update_source(
        &self,
        source_key: &str,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        if input.source_key != source_key {
            return Err(anyhow!("source_key cannot be changed"));
        }

        self.upsert_source_connection_for_source(input).await?;
        let _guard = self.acquire_lock(source_key).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.update_source(source_key, &source).await?;
        self.reload_sources().await?;
        self.source_store
            .get_source(source_key)
            .await?
            .with_context(|| format!("missing source {source_key}"))
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        let chunk_ids = self.source_store.list_source_chunk_ids(source_key).await?;
        self.index.delete_points(&chunk_ids).await?;
        let deleted = self.source_store.delete_source(source_key).await?;
        if !deleted {
            return Err(anyhow!("unknown source {source_key}"));
        }
        self.reload_sources().await?;
        Ok(())
    }

    async fn upsert_source_connection_for_source(&self, input: &SourceConfigInput) -> Result<()> {
        let stored = self
            .resolve_source_connection(&input.connection, input.database_url.clone())
            .await?;
        self.db.save_source_connection(&stored).await?;
        self.reload_sources().await?;
        Ok(())
    }
}
