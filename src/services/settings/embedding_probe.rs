//! The embedding provider connection probe and the capability discovery.
//!
//! Both are non-persisting: a probe resolves a credential, builds a provider
//! from the submitted values, and embeds one synthetic string; a capability
//! request reads the provider's own capability document. No error path on
//! either carries the API key, and neither writes a row.

use anyhow::Result;

use super::SettingsService;
use crate::contracts::settings::{
    EmbeddingBatchingSettings, EmbeddingCapabilitiesResponse, EmbeddingCapabilitiesSource,
    TestRuntimeEmbeddingRequest,
};
use crate::embedding::embedding_batch::EmbeddingBatchLimits;
use crate::embedding::embedding_capabilities::{self, EmbeddingCapabilities};
use crate::support::normalize::normalize_optional_string;

/// The synthetic string a probe embeds.
const PROBE_TEXT: &str = "context69 embedding connection test";

impl SettingsService {
    /// The credential a non-persisting probe may use.
    ///
    /// A submitted key is used for that draft only. The stored one is used only
    /// when the draft points at the same origin as the stored configuration, so
    /// a stored credential never leaves the origin it belongs to.
    async fn probe_credential(
        &self,
        base_url: &str,
        submitted: Option<String>,
    ) -> Result<Option<String>> {
        if let Some(supplied) = normalize_optional_string(submitted) {
            return Ok(Some(supplied));
        }
        let stored = self.db.get_runtime_settings().await?;
        let stored_origin = stored
            .as_ref()
            .and_then(|settings| embedding_capabilities::origin(&settings.embedding.base_url));
        if stored_origin.is_none() || stored_origin != embedding_capabilities::origin(base_url) {
            return Ok(None);
        }
        self.embedding_secrets.resolve().await
    }

    /// The batching limits a non-persisting probe plans against.
    ///
    /// They are the stored ones when a configuration exists: the limits belong
    /// to the deployment, not to the draft.
    async fn probe_batching(&self) -> Result<EmbeddingBatchingSettings> {
        Ok(self
            .db
            .get_runtime_settings()
            .await?
            .map(|settings| settings.embedding.batching)
            .unwrap_or_default())
    }

    pub async fn test_embedding_connection(
        &self,
        request: &context69_contracts::settings::TestRuntimeEmbeddingRequest,
    ) -> Result<()> {
        // A probe never writes. A supplied key is used as given, and otherwise
        // the stored one is resolved through the store, which fails closed
        // rather than reporting no credential.
        let api_key = self
            .probe_credential(&request.base_url, request.api_key.clone())
            .await?;
        let batching = self.probe_batching().await?;
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
                },
            )?;
            query_provider
                .embed_texts(&[PROBE_TEXT.to_string()])
                .await?;
        }
        Ok(())
    }

    /// Reads the capability document of the submitted embedding provider.
    ///
    /// The request is non-persisting and never fails the caller's
    /// configuration: a provider that does not publish capabilities, or that
    /// refuses the request, is reported as `unknown` with the manual limits in
    /// effect. The discovered facts are cached for the installed provider
    /// configuration, so the request planner can tighten the manual limits
    /// without ever asking again.
    pub async fn embedding_capabilities(
        &self,
        request: &TestRuntimeEmbeddingRequest,
    ) -> Result<EmbeddingCapabilitiesResponse> {
        let manual = self.probe_batching().await?;
        let document_model = request.model.trim().to_string();
        let query_model = normalize_optional_string(request.query_model.clone())
            .unwrap_or_else(|| document_model.clone());
        let base_url = request.base_url.trim().to_string();

        let api_key = self
            .probe_credential(&base_url, request.api_key.clone())
            .await?;
        let discovered = embedding_capabilities::discover(&base_url, api_key.as_deref()).await;
        match discovered {
            Ok(capabilities) => {
                embedding_capabilities::remember(&base_url, capabilities.clone());
                Ok(capabilities_response(
                    EmbeddingCapabilitiesSource::Discovered,
                    base_url,
                    document_model,
                    query_model,
                    manual,
                    Some(&capabilities),
                    None,
                ))
            }
            Err(error) => Ok(capabilities_response(
                EmbeddingCapabilitiesSource::Unknown,
                base_url,
                document_model,
                query_model,
                manual,
                None,
                Some(error.to_string()),
            )),
        }
    }
}

/// The typed answer of one capability request.
///
/// The advertised facts are optional: a provider that publishes none of them
/// still yields the manual limits, so the caller always learns which limits are
/// in effect.
fn capabilities_response(
    source: EmbeddingCapabilitiesSource,
    base_url: String,
    document_model: String,
    query_model: String,
    manual: EmbeddingBatchingSettings,
    advertised: Option<&EmbeddingCapabilities>,
    reason: Option<String>,
) -> EmbeddingCapabilitiesResponse {
    let manual_limits = EmbeddingBatchLimits::from_settings(&manual);
    let effective_limits = embedding_capabilities::effective_limits(&manual_limits, advertised);
    let advertised_tokens_per_input_limit = advertised
        .and_then(|capabilities| capabilities.tokens.as_ref())
        .and_then(|tokens| tokens.per_input_limit);
    let role_aliases = advertised
        .map(|capabilities| capabilities.aliases.clone())
        .filter(|aliases| !aliases.is_empty());
    let supported_dimensions = advertised
        .and_then(|capabilities| capabilities.dimensions.as_ref())
        .map(|dimensions| dimensions.supported.clone())
        .filter(|dimensions| !dimensions.is_empty());
    EmbeddingCapabilitiesResponse {
        source,
        base_url,
        document_model,
        query_model,
        manual_limits: manual,
        effective_limits: effective_limits.to_settings(),
        advertised_model: advertised.and_then(|capabilities| capabilities.model.clone()),
        role_aliases,
        supported_dimensions,
        advertised_max_inputs_per_request: advertised
            .and_then(|capabilities| capabilities.input.as_ref())
            .and_then(|input| input.max_inputs_per_request),
        advertised_tokens_per_input_limit,
        token_limit_enforced_by_server: advertised_tokens_per_input_limit.is_some(),
        tokens_include_role_prefix: advertised
            .and_then(EmbeddingCapabilities::includes_role_prefix),
        tokens_include_special_tokens: advertised
            .and_then(EmbeddingCapabilities::includes_special_tokens),
        internal_batch_size: advertised.and_then(|capabilities| capabilities.batch_size),
        max_concurrent_encodes: advertised
            .and_then(|capabilities| capabilities.max_concurrent_encodes),
        reason,
    }
}
