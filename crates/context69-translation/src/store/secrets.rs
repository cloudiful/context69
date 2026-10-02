use anyhow::Result;
use context69_secret_store::{SecretPurpose, SecretStore, key_names};

use super::codec::clean;

/// The shared `provider_key = 'llm'` API key, owned by translation and read by
/// extraction.
///
/// Both crates bind the row to the singleton
/// [`SecretPurpose::TranslationProviderApiKey`], so the value written here is
/// the one extraction reads. The store is the only representation: the
/// `translation_provider_settings.api_key` column belongs to the sibling
/// providers, whose credentials no purpose owns, and the shared row never writes
/// it.
#[derive(Debug, Clone)]
pub(super) struct ProviderApiKey {
    store: SecretStore,
}

impl ProviderApiKey {
    pub(super) fn new(store: SecretStore) -> Self {
        Self { store }
    }

    const KEY_NAME: &'static str = key_names::TRANSLATION_PROVIDER_API_KEY;

    /// The shared key, or nothing when it is not configured. A stored row that
    /// cannot be opened fails instead of reporting an empty credential.
    pub(super) async fn resolve(&self) -> Result<Option<String>> {
        let Some(stored) = self
            .store
            .get(SecretPurpose::TranslationProviderApiKey, Self::KEY_NAME)
            .await?
        else {
            return Ok(None);
        };
        Ok(clean(Some(
            String::from_utf8_lossy(stored.expose()).as_ref(),
        )))
    }

    /// Metadata-only presence: a stored row answers for itself, so this never
    /// needs a master key.
    pub(super) async fn present(&self) -> Result<bool> {
        Ok(self
            .store
            .has(SecretPurpose::TranslationProviderApiKey, Self::KEY_NAME)
            .await?)
    }

    /// Writes a non-blank value. `None` or a blank value is a Keep: nothing is
    /// written, so an existing credential survives an api-key-less update.
    pub(super) async fn write(&self, value: Option<&str>) -> Result<()> {
        if let Some(value) = clean(value) {
            self.store
                .write(
                    SecretPurpose::TranslationProviderApiKey,
                    Self::KEY_NAME,
                    value.as_bytes(),
                )
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_provider_key_is_a_singleton_purpose() {
        assert_eq!(
            SecretPurpose::TranslationProviderApiKey.singleton_key_name(),
            Some(key_names::TRANSLATION_PROVIDER_API_KEY)
        );
        assert!(SecretPurpose::TranslationProviderApiKey.is_singleton());
        assert_eq!(
            SecretPurpose::TranslationProviderApiKey.as_str(),
            "translation.api_key"
        );
    }

    #[test]
    fn the_shared_key_is_bound_to_its_catalogue_key_name() {
        assert_eq!(
            ProviderApiKey::KEY_NAME,
            key_names::TRANSLATION_PROVIDER_API_KEY
        );
        assert_eq!(ProviderApiKey::KEY_NAME, "translation_provider_api_key");
    }
}
