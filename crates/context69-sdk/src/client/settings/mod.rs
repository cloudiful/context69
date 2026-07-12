mod docling;
mod runtime;
mod search;
mod translation;

use super::Context69Client;

pub use docling::DoclingSettingsApi;
pub use runtime::RuntimeSettingsApi;
pub use search::SearchSettingsApi;
pub use translation::TranslationSettingsApi;

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

    pub fn docling(&self) -> DoclingSettingsApi<'a> {
        DoclingSettingsApi::new(self.client)
    }

    pub fn search(&self) -> SearchSettingsApi<'a> {
        SearchSettingsApi::new(self.client)
    }

    pub fn translation(&self) -> TranslationSettingsApi<'a> {
        TranslationSettingsApi::new(self.client)
    }
}
