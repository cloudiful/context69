//! The embedding provider connection probe.
//!
//! A non-persisting reachability check: it resolves a credential, builds a
//! provider from the submitted values, and embeds one synthetic string. No error
//! path on it carries the API key.

use anyhow::Result;

use super::SettingsService;
use crate::support::normalize::normalize_optional_string;

impl SettingsService {
    pub async fn test_embedding_connection(
        &self,
        request: &context69_contracts::settings::TestRuntimeEmbeddingRequest,
    ) -> Result<()> {
        // A probe never writes. A supplied key is used as given, and otherwise
        // the stored one is resolved through the store, which fails closed
        // rather than reporting no credential.
        let api_key = match normalize_optional_string(request.api_key.clone()) {
            Some(supplied) => Some(supplied),
            None => self.embedding_secrets.resolve().await?,
        };
        // Construction performs no I/O; the probe embeds one synthetic string.
        // Every error path carries endpoint/model only, never the key.
        let provider = crate::embedding::build_probe_provider(
            &request.base_url,
            &request.model,
            request.dimensions,
            request.timeout_secs,
            api_key,
        )?;
        provider
            .embed_texts(&["context69 embedding connection test".to_string()])
            .await?;
        Ok(())
    }
}
