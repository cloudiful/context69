//! Secret handling for the settings API keys.
//!
//! The embedding, search/rerank, and Docling VLM API keys are the first three
//! categories routed through the shared store. This module is the whole of that
//! routing, kept out of the settings service so the service does not grow a
//! second responsibility.
//!
//! # The transition contract
//!
//! The legacy plaintext columns still exist and are still written, so a
//! rollback to the previous release loses nothing. Reads are therefore ordered:
//!
//! 1. **store first** — a value in the store wins, sealed and purpose-bound;
//! 2. **legacy second** — a key that has not been migrated yet is still read
//!    from its plaintext column, so no deployment breaks before the backfill;
//! 3. **fail closed** — if a store row *exists* and cannot be opened (a sealed
//!    row with no usable master key, a wrong key version, a tampered row), the
//!    call fails. It never falls back to the legacy column, because doing so
//!    would silently serve a stale credential as if it were the current one.
//!
//! Presence is answered separately, and metadata-only: a `has_api_key` response
//! must stay truthful on a deployment that cannot decrypt, so it asks the store
//! whether a row exists without ever opening it, and only then considers the
//! legacy column.
//!
//! Writes go to both places, which is what makes the transition rollback-safe:
//! the store gets the sealed value it will own after the backfill, and the
//! legacy column keeps receiving it until the columns are removed.

use anyhow::Result;

use crate::{
    contracts::SecretPatch,
    services::secret_store::{SecretPurpose, SecretStore, SecretStoreError, key_names},
    support::normalize::normalize_optional_string,
};

/// The settings-side view of the shared store.
///
/// Holds the store and the purpose/key-name pair for one settings category, so
/// no caller names a secret by string literal.
#[derive(Clone, Debug)]
pub struct SettingsSecrets {
    store: SecretStore,
    purpose: SecretPurpose,
    key_name: &'static str,
}

impl SettingsSecrets {
    /// Binds one settings category to the shared store.
    pub fn new(store: SecretStore, purpose: SecretPurpose, key_name: &'static str) -> Self {
        Self {
            store,
            purpose,
            key_name,
        }
    }

    /// The embedding provider API key.
    pub fn embedding(store: SecretStore) -> Self {
        Self::new(
            store,
            SecretPurpose::EmbeddingApiKey,
            key_names::EMBEDDING_API_KEY,
        )
    }

    /// The search / rerank provider API key.
    pub fn search(store: SecretStore) -> Self {
        Self::new(
            store,
            SecretPurpose::SearchApiKey,
            key_names::SEARCH_API_KEY,
        )
    }

    /// The Docling VLM provider API key.
    pub fn docling(store: SecretStore) -> Self {
        Self::new(
            store,
            SecretPurpose::DoclingVlmApiKey,
            key_names::DOCLING_VLM_API_KEY,
        )
    }

    /// The effective value: the store when it holds one, otherwise the legacy
    /// column.
    ///
    /// Fails closed — see the module contract.
    pub async fn resolve(&self, legacy: Option<String>) -> Result<Option<String>> {
        match self.store.get(self.purpose, self.key_name).await? {
            Some(stored) => Ok(normalize_optional_string(Some(
                String::from_utf8_lossy(stored.expose()).into_owned(),
            ))),
            None => Ok(normalize_optional_string(legacy)),
        }
    }

    /// Whether this category is configured, without opening anything.
    ///
    /// A store row answers for itself, including a sealed row this deployment
    /// cannot read, so a `has_api_key` response never depends on the master key.
    pub async fn is_present(&self, legacy: Option<&str>) -> Result<bool> {
        if self
            .store
            .has(self.purpose, self.key_name)
            .await
            .map_err(|error| self.failure(error))?
        {
            return Ok(true);
        }
        Ok(normalize_optional_string(legacy.map(str::to_string)).is_some())
    }

    /// Resolves the current value and folds one tri-state patch onto it.
    ///
    /// This is the shape every settings write needs: read what is in effect,
    /// fold the patch onto it, and keep the result. Doing it in one place is
    /// what keeps a caller from patching a stale value and silently losing a
    /// credential. Nothing is stored here — [`Self::commit`] does that — so a
    /// caller can still validate the merged settings before anything is
    /// written.
    pub async fn resolve_and_merge(
        &self,
        patch: &SecretPatch,
        legacy: Option<String>,
    ) -> Result<Option<String>> {
        if matches!(patch, SecretPatch::Clear) {
            // A clear removes the value whatever it currently is, so nothing has
            // to be resolved — and a sealed key this deployment cannot open must
            // not be the thing that prevents it from being removed.
            return Ok(None);
        }
        let current = self.resolve(legacy).await?;
        Ok(merged_api_key(patch, current))
    }

    /// Resolves the effective value, folds `patch` onto it, builds the settings
    /// value with the merged key, validates it, and only then commits `patch`.
    ///
    /// The order is the point: a request that fails validation must not change
    /// the stored key. Every write path shares the sequence, so no call site can
    /// get it wrong on its own.
    pub async fn stage<T>(
        &self,
        patch: &SecretPatch,
        legacy: Option<String>,
        build: impl FnOnce(Option<String>) -> T,
        validate: impl FnOnce(&T) -> Result<()>,
    ) -> Result<T> {
        let merged = self.resolve_and_merge(patch, legacy).await?;
        let candidate = build(merged);
        validate(&candidate)?;
        self.commit(patch).await?;
        Ok(candidate)
    }

    /// Applies one tri-state patch to the store.
    ///
    /// `Keep` and a blank `Set` leave the stored value alone, `Set` writes it,
    /// and `Clear` removes it. This is the only step that mutates the store.
    pub async fn commit(&self, patch: &SecretPatch) -> Result<()> {
        let outcome = match patch {
            SecretPatch::Clear => self
                .store
                .delete(self.purpose, self.key_name)
                .await
                .map(|_| ()),
            SecretPatch::Set(value) => match normalize_optional_string(Some(value.clone())) {
                Some(value) => self
                    .store
                    .write(self.purpose, self.key_name, value.as_bytes())
                    .await
                    .map(|_| ()),
                // A blank Set is a Keep: it must not clear what is stored.
                None => Ok(()),
            },
            SecretPatch::Keep => Ok(()),
        };
        outcome.map_err(|error| self.failure(error))
    }

    /// Wraps a store failure with the owning purpose, so an operator sees which
    /// credential is unavailable. A missing master key keeps its own advice; no
    /// variant ever carries a value.
    fn failure(&self, error: SecretStoreError) -> anyhow::Error {
        secret_error(self.purpose, error)
    }
}

/// The patch a plain optional value key field maps to: an explicit value
/// replaces and an absent one keeps. Such a field cannot express a clear, which
/// is the contract it already had.
pub(crate) fn optional_key_patch(requested: Option<String>) -> SecretPatch {
    match normalize_optional_string(requested) {
        Some(api_key) => SecretPatch::Set(api_key),
        None => SecretPatch::Keep,
    }
}

/// The patch a Docling VLM update maps to.
///
/// A submitted key replaces the stored one, and a request that keeps the base
/// URL keeps the key. A request that drops the base URL drops the key with it,
/// which is the mapping the settings contract already documents.
pub(crate) fn docling_vlm_patch(requested: Option<String>, base_url_present: bool) -> SecretPatch {
    match normalize_optional_string(requested) {
        Some(api_key) => SecretPatch::Set(api_key),
        None if base_url_present => SecretPatch::Keep,
        None => SecretPatch::Clear,
    }
}

/// Folds a tri-state patch onto the value currently in effect.
///
/// `Keep` preserves it, `Set` replaces it, and `Clear` removes it. A blank `Set`
/// normalizes back to `Keep`, which is the legacy whitespace parity the wire
/// contract already documents.
pub(crate) fn merged_api_key(patch: &SecretPatch, current: Option<String>) -> Option<String> {
    match patch {
        SecretPatch::Clear => None,
        SecretPatch::Set(value) => normalize_optional_string(Some(value.clone())).or(current),
        SecretPatch::Keep => current,
    }
}

/// Maps a store failure onto the settings boundary.
///
/// A missing master key is a configuration failure the operator has to fix, so
/// it keeps its own message rather than being flattened into a generic error.
pub(crate) fn secret_error(purpose: SecretPurpose, error: SecretStoreError) -> anyhow::Error {
    match error {
        SecretStoreError::MasterKeyNotConfigured => anyhow::anyhow!(
            "{purpose} credential is stored encrypted but no secret_store.master_key is configured"
        ),
        other => anyhow::anyhow!("{purpose} credential is unavailable: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SettingsSecrets, docling_vlm_patch, merged_api_key, optional_key_patch, secret_error,
    };
    use crate::{
        contracts::SecretPatch,
        services::secret_store::{SecretPurpose, SecretStore, SecretStoreError},
    };

    /// A store over a lazily connected pool. It is never connected: the paths
    /// asserted here decide before any statement runs.
    fn unkeyed_store() -> SecretStore {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/unused")
            .expect("a lazy pool needs no server");
        SecretStore::new(context69_secret_store::SecretDatabase::new(pool), None, 1)
            .expect("an unkeyed store cannot fail to build")
    }

    fn patch(kind: SecretPatch) -> Option<String> {
        merged_api_key(&kind, Some("old".to_string()))
    }

    #[test]
    fn the_tri_state_patches_preserve_keep_set_and_clear() {
        assert_eq!(patch(SecretPatch::Keep), Some("old".to_string()));
        assert_eq!(
            patch(SecretPatch::Set("new".to_string())),
            Some("new".to_string())
        );
        assert_eq!(patch(SecretPatch::Clear), None);
        // A blank Set normalizes to Keep (legacy whitespace parity).
        assert_eq!(
            patch(SecretPatch::Set("  ".to_string())),
            Some("old".to_string())
        );
        // Keep/Set/Clear from no current value.
        assert_eq!(merged_api_key(&SecretPatch::Keep, None), None);
        assert_eq!(
            merged_api_key(&SecretPatch::Set("new".to_string()), None),
            Some("new".to_string())
        );
        assert_eq!(merged_api_key(&SecretPatch::Clear, None), None);
    }

    #[test]
    fn an_optional_key_field_maps_to_a_keep_or_a_set() {
        assert_eq!(
            optional_key_patch(Some("key".to_string())),
            SecretPatch::Set("key".to_string())
        );
        assert_eq!(
            optional_key_patch(Some("  ".to_string())),
            SecretPatch::Keep
        );
        assert_eq!(optional_key_patch(None), SecretPatch::Keep);
    }

    #[test]
    fn a_docling_update_maps_to_set_keep_or_clear() {
        assert_eq!(
            docling_vlm_patch(Some("key".to_string()), true),
            SecretPatch::Set("key".to_string())
        );
        // A blank key with a base URL keeps what is stored.
        assert_eq!(
            docling_vlm_patch(Some("  ".to_string()), true),
            SecretPatch::Keep
        );
        assert_eq!(docling_vlm_patch(None, true), SecretPatch::Keep);
        // Dropping the base URL drops the key with it.
        assert_eq!(docling_vlm_patch(None, false), SecretPatch::Clear);
        assert_eq!(
            docling_vlm_patch(Some("key".to_string()), false),
            SecretPatch::Set("key".to_string())
        );
    }

    #[tokio::test]
    async fn clearing_a_key_needs_no_master_key_and_opens_nothing() {
        // A clear removes whatever is stored, so it must not require opening —
        // or even reaching — the row it removes.
        let secrets = SettingsSecrets::docling(unkeyed_store());
        assert_eq!(
            secrets
                .resolve_and_merge(&SecretPatch::Clear, Some("legacy".to_string()))
                .await
                .expect("a clear resolves without a database"),
            None
        );
    }

    #[test]
    fn a_missing_master_key_is_reported_as_a_configuration_failure() {
        let error = secret_error(
            SecretPurpose::SearchApiKey,
            SecretStoreError::MasterKeyNotConfigured,
        );
        let message = error.to_string();
        assert!(message.contains("search.api_key"), "{message}");
        assert!(message.contains("secret_store.master_key"), "{message}");

        // Every other store failure keeps the store's own reason, still naming
        // the purpose and never a value.
        let error = secret_error(
            SecretPurpose::EmbeddingApiKey,
            SecretStoreError::Cipher(
                context69_secret_store::crypto::SecretCipherError::AuthenticationFailed,
            ),
        );
        let message = error.to_string();
        assert!(message.contains("embedding.api_key"), "{message}");
        assert!(message.contains("authentication"), "{message}");
    }
}
