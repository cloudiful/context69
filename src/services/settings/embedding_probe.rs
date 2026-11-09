//! The embedding provider connection probe.
//!
//! The probe is non-persisting: it resolves a credential, builds a provider
//! from the submitted values, and embeds one synthetic string. No error path
//! carries the API key, and it writes no row.

use anyhow::Result;

use super::SettingsService;
use crate::contracts::settings::{
    EmbeddingBatchingSettings, TestRuntimeEmbeddingPurpose, TestRuntimeEmbeddingRequest,
};
use crate::support::normalize::normalize_optional_string;

/// The synthetic string a probe embeds.
const PROBE_TEXT: &str = "context69 embedding connection test";

/// The origin of one base URL.
///
/// Only the scheme and the authority are used, so a base URL that carries a
/// path prefix or a query does not change the origin.
fn origin(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    let (scheme, rest) = trimmed.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

impl SettingsService {
    /// The credential a non-persisting probe may use.
    ///
    /// A submitted key is used for that draft only. The stored one is used only
    /// when the draft points at the same origin as the stored configuration, so
    /// a stored credential never leaves the origin it belongs to.
    async fn probe_credential(
        &self,
        purpose: TestRuntimeEmbeddingPurpose,
        base_url: &str,
        submitted: Option<String>,
    ) -> Result<Option<String>> {
        if let Some(supplied) = normalize_optional_string(submitted) {
            return Ok(Some(supplied));
        }
        match purpose {
            TestRuntimeEmbeddingPurpose::Document => {
                let stored = self.db.get_runtime_settings().await?;
                let stored_origin = stored
                    .as_ref()
                    .and_then(|settings| origin(&settings.embedding.base_url));
                if stored_origin.is_none() || stored_origin != origin(base_url) {
                    return Ok(None);
                }
                self.embedding_secrets.resolve().await
            }
            TestRuntimeEmbeddingPurpose::Code => {
                // A code probe resolves only the code configuration and code
                // credential. It never reads or opens the document key.
                let stored = self.db.get_runtime_code_embedding_settings().await?;
                let stored_origin = stored
                    .as_ref()
                    .and_then(|settings| origin(&settings.base_url));
                if stored_origin.is_none() || stored_origin != origin(base_url) {
                    return Ok(None);
                }
                self.code_embedding_secrets.resolve().await
            }
        }
    }

    /// The batching limits a non-persisting probe plans against.
    ///
    /// They are the stored limits of the selected purpose when a configuration
    /// exists: the limits belong to the deployment, not to the draft.
    async fn probe_batching(
        &self,
        purpose: TestRuntimeEmbeddingPurpose,
    ) -> Result<EmbeddingBatchingSettings> {
        Ok(match purpose {
            TestRuntimeEmbeddingPurpose::Document => self
                .db
                .get_runtime_settings()
                .await?
                .map(|settings| settings.embedding.batching)
                .unwrap_or_default(),
            TestRuntimeEmbeddingPurpose::Code => self
                .db
                .get_runtime_code_embedding_settings()
                .await?
                .map(|settings| settings.batching)
                .unwrap_or_default(),
        })
    }

    pub async fn test_embedding_connection(
        &self,
        request: &TestRuntimeEmbeddingRequest,
    ) -> Result<()> {
        // A probe never writes. A supplied key is used as given, and otherwise
        // the stored key of the selected purpose is resolved through the store,
        // which fails closed rather than reporting no credential.
        let api_key = self
            .probe_credential(request.purpose, &request.base_url, request.api_key.clone())
            .await?;
        let batching = self.probe_batching(request.purpose).await?;
        // Construction performs no I/O; the probe embeds one synthetic string.
        // Every error path carries endpoint/model only, never the key.
        let provider =
            crate::embedding::build_probe_provider_with_config(crate::config::EmbeddingConfig {
                base_url: request.base_url.trim().to_string(),
                api_key: api_key.clone(),
                model: request.model.trim().to_string(),
                query_model: None,
                dimensions: request.dimensions,
                timeout: std::time::Duration::from_secs(request.timeout_secs),
                batching,
                input_policy: Default::default(),
            })?;
        provider.embed_texts(&[PROBE_TEXT.to_string()]).await?;

        // A second, distinct query model is a second provider contract, so it is
        // probed as well: a reachable document model says nothing about the
        // alias that embeds queries.
        let document_model = request.model.trim();
        let query_model = normalize_optional_string(request.query_model.clone());
        if let Some(query_model) = query_model
            && query_model != document_model
        {
            let query_provider = crate::embedding::build_probe_provider_with_config(
                crate::config::EmbeddingConfig {
                    base_url: request.base_url.trim().to_string(),
                    api_key,
                    model: query_model.clone(),
                    query_model: None,
                    dimensions: request.dimensions,
                    timeout: std::time::Duration::from_secs(request.timeout_secs),
                    batching,
                    input_policy: Default::default(),
                },
            )?;
            query_provider
                .embed_texts(&[PROBE_TEXT.to_string()])
                .await?;
        }
        Ok(())
    }
}
