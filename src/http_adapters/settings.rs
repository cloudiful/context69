use async_trait::async_trait;
use context69_settings_http::SettingsApi;

use crate::services::settings::SettingsService;

#[derive(Clone)]
pub struct SettingsApiAdapter {
    service: SettingsService,
}

impl SettingsApiAdapter {
    pub fn new(service: SettingsService) -> Self {
        Self { service }
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

    async fn list_provider_accounts(
        &self,
    ) -> anyhow::Result<Vec<crate::contracts::ProviderAccountResponse>> {
        self.service.list_provider_accounts().await
    }

    async fn upsert_provider_account(
        &self,
        request: &crate::contracts::UpsertProviderAccountRequest,
    ) -> anyhow::Result<crate::contracts::ProviderAccountResponse> {
        self.service.upsert_provider_account(request).await
    }

    async fn delete_provider_account(&self, account_key: &str) -> anyhow::Result<()> {
        self.service.delete_provider_account(account_key).await
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
