mod docling;
mod providers;
mod runtime;
mod search;

use super::Context69Client;

pub use docling::DoclingSettingsApi;
pub use providers::{ProviderAccountApi, ProviderAccountsApi};
pub use runtime::RuntimeSettingsApi;
pub use search::SearchSettingsApi;

pub struct SettingsApi<'a> {
    client: &'a Context69Client,
}

impl<'a> SettingsApi<'a> {
    pub(crate) fn new(client: &'a Context69Client) -> Self {
        Self { client }
    }

    pub fn runtime(&self) -> RuntimeSettingsApi<'a> {
        RuntimeSettingsApi::new(self.client)
    }

    pub fn provider_accounts(&self) -> ProviderAccountsApi<'a> {
        ProviderAccountsApi::new(self.client)
    }

    pub fn provider_account(&self, account_key: impl Into<String>) -> ProviderAccountApi<'a> {
        ProviderAccountApi::new(self.client, account_key.into())
    }

    pub fn docling(&self) -> DoclingSettingsApi<'a> {
        DoclingSettingsApi::new(self.client)
    }

    pub fn search(&self) -> SearchSettingsApi<'a> {
        SearchSettingsApi::new(self.client)
    }
}
