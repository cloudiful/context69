use async_trait::async_trait;
use context69_settings_http::SettingsApi;

use crate::services::settings::SettingsService;
use crate::services::sync::SyncService;

#[derive(Clone)]
pub struct SettingsApiAdapter {
    service: SettingsService,
    sync: SyncService,
}

impl SettingsApiAdapter {
    pub fn new(service: SettingsService, sync: SyncService) -> Self {
        Self { service, sync }
    }
}

#[async_trait]
impl SettingsApi for SettingsApiAdapter {
    async fn get_runtime_settings(
        &self,
    ) -> anyhow::Result<crate::contracts::RuntimeSettingsResponse> {
        self.service.get_runtime_settings().await
    }

    async fn update_runtime_settings(
        &self,
        request: &crate::contracts::UpdateRuntimeSettingsRequest,
    ) -> anyhow::Result<crate::contracts::RuntimeSettingsResponse> {
        self.service.update_runtime_settings(request).await
    }

    async fn test_s3_connection(
        &self,
        request: &crate::contracts::UpdateRuntimeS3Settings,
    ) -> anyhow::Result<()> {
        self.service.test_s3_connection(request).await
    }

    async fn test_valkey_connection(
        &self,
        request: &crate::contracts::TestRuntimeValkeyRequest,
    ) -> anyhow::Result<()> {
        self.service.test_valkey_connection(request).await
    }

    async fn get_vector_index_rebuild_status(
        &self,
    ) -> anyhow::Result<crate::contracts::VectorIndexRebuildStatus> {
        Ok(self.sync.vector_index_rebuild_status().await)
    }

    async fn start_vector_index_rebuild(
        &self,
    ) -> anyhow::Result<crate::contracts::VectorIndexRebuildStatus> {
        self.sync.start_vector_index_rebuild().await
    }

    async fn get_docling_settings(
        &self,
    ) -> anyhow::Result<crate::contracts::DoclingSettingsResponse> {
        self.service.get_docling_settings().await
    }

    async fn update_docling_settings(
        &self,
        request: &crate::contracts::UpdateDoclingSettingsRequest,
    ) -> anyhow::Result<crate::contracts::DoclingSettingsResponse> {
        self.service.update_docling_settings(request).await
    }

    async fn get_search_settings(
        &self,
    ) -> anyhow::Result<crate::contracts::SearchSettingsResponse> {
        self.service.get_search_settings().await
    }

    async fn update_search_settings(
        &self,
        request: &crate::contracts::UpdateSearchSettingsRequest,
    ) -> anyhow::Result<crate::contracts::SearchSettingsResponse> {
        self.service.update_search_settings(request).await
    }
}
