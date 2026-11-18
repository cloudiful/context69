//! The embedding provider connection probe.
//!
//! The probe is non-persisting: it resolves a credential, builds a provider
//! from the submitted values, and embeds one synthetic string. No error path
//! carries the API key, and it writes no row.
//!
//! The probe answers the question the save of the same draft would answer: an
//! optional channel inherits a field it omits from the unsaved document draft,
//! and the document block resolves its own complete draft without inheriting
//! anything. The persisted row contributes only credential origins, never a
//! value under test.

use anyhow::Result;

use super::SettingsService;
use crate::contracts::settings::{
    TestRuntimeEmbeddingKeyIntent, TestRuntimeEmbeddingPurpose, TestRuntimeEmbeddingRequest,
};
use crate::domain_errors::DomainError;
use crate::services::settings::inheritance::{
    ChannelCredentialChoice, ChannelCredentialInput, DocumentEmbeddingSource, InheritedCredential,
    Origin, channel_credential_choice, code_overrides_from_probe, image_overrides_from_probe,
    inherited_credential, origin_of, resolve_code_settings, resolve_document_probe,
    resolve_image_settings, submitted_probe_key,
};

/// The synthetic string a probe embeds.
const PROBE_TEXT: &str = "context69 embedding connection test";

/// A 1x1 transparent PNG sent by the image provider probe.
const PROBE_IMAGE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

impl SettingsService {
    /// The document configuration a probe inherits from.
    ///
    /// The unsaved document draft is the source when the request carries one:
    /// a probe must answer for the draft a save would store, not for the
    /// persisted row. A request without a draft falls back to the stored
    /// document configuration.
    async fn probe_document_source(
        &self,
        request: &TestRuntimeEmbeddingRequest,
    ) -> Result<DocumentEmbeddingSource> {
        let carries_draft = request.document_base_url.is_some()
            || request.document_model.is_some()
            || request.document_query_model.is_some()
            || request.document_dimensions.is_some()
            || request.document_timeout_secs.is_some();
        if carries_draft {
            return Ok(DocumentEmbeddingSource {
                base_url: request
                    .document_base_url
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                model: request
                    .document_model
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                query_model: request
                    .document_query_model
                    .as_deref()
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                    .map(str::to_string),
                dimensions: request.document_dimensions.unwrap_or(0) as u64,
                timeout_secs: request.document_timeout_secs.unwrap_or(0),
                // The document draft carries no credential: a probe never
                // carries a user secret it did not submit itself.
                api_key: None,
            });
        }
        let stored = self.db.get_runtime_settings().await?;
        Ok(stored
            .as_ref()
            .map(|runtime| DocumentEmbeddingSource::from_stored(&runtime.embedding))
            .unwrap_or_default())
    }

    /// The credential a non-persisting probe may use.
    ///
    /// A submitted key is used for that call only. The stored own-purpose
    /// credential is used only at the origin it was configured for, so a stored
    /// secret never travels to another endpoint. The document credential is
    /// used only at the document origin, the persisted one authorizes the
    /// stored secret, and the unsaved document draft only authorizes the key it
    /// submits itself. `Inherited` skips the own-purpose credential entirely: a
    /// pending clear is probed against the credential that would be in effect
    /// after it.
    async fn probe_credential(
        &self,
        request: &TestRuntimeEmbeddingRequest,
        effective_base_url: &str,
        document: &DocumentEmbeddingSource,
    ) -> Result<Option<String>> {
        let intent = request.key_intent.unwrap_or_default();
        if let Some(supplied) = submitted_probe_key(intent, request.api_key.as_deref()) {
            return Ok(Some(supplied));
        }
        let target = origin_of(effective_base_url);
        let stored_document_origin = self.stored_document_origin().await?;
        match request.purpose {
            TestRuntimeEmbeddingPurpose::Document => {
                // The document probe uses the stored document credential only
                // when the endpoint under test is the stored document origin.
                if stored_document_origin.is_none() || stored_document_origin != target {
                    return Ok(None);
                }
                self.embedding_secrets.resolve().await
            }
            TestRuntimeEmbeddingPurpose::Code => {
                if intent == TestRuntimeEmbeddingKeyIntent::Stored {
                    let key = self.code_embedding_secrets.resolve().await?;
                    let stored_endpoint = self
                        .db
                        .get_runtime_code_embedding_settings()
                        .await?
                        .and_then(|stored| stored.base_url);
                    let document_key = request.document_api_key.clone().unwrap_or_default();
                    let draft_document_origin = origin_of(&document.base_url);
                    let choice = channel_credential_choice(ChannelCredentialInput {
                        own_key_present: key.is_some(),
                        stored_channel_endpoint: stored_endpoint.as_deref(),
                        stored_document_origin: stored_document_origin.as_ref(),
                        target: target.as_ref(),
                        document_key: &document_key,
                        draft_document_origin: draft_document_origin.as_ref(),
                    });
                    match choice {
                        ChannelCredentialChoice::StoredPurpose => return Ok(key),
                        ChannelCredentialChoice::UnreachableOwnCredential => {
                            return Err(stored_credential_origin_mismatch("code"));
                        }
                        ChannelCredentialChoice::Document(choice) => {
                            return self.resolve_inherited_credential(choice).await;
                        }
                    }
                }
                self.inherited_document_credential(
                    request,
                    target.as_ref(),
                    document,
                    stored_document_origin.as_ref(),
                )
                .await
            }
            TestRuntimeEmbeddingPurpose::Image => {
                if intent == TestRuntimeEmbeddingKeyIntent::Stored {
                    let key = self.image_embedding_secrets.resolve().await?;
                    let stored_endpoint = self
                        .db
                        .get_runtime_image_embedding_settings()
                        .await?
                        .and_then(|stored| stored.base_url);
                    let document_key = request.document_api_key.clone().unwrap_or_default();
                    let draft_document_origin = origin_of(&document.base_url);
                    let choice = channel_credential_choice(ChannelCredentialInput {
                        own_key_present: key.is_some(),
                        stored_channel_endpoint: stored_endpoint.as_deref(),
                        stored_document_origin: stored_document_origin.as_ref(),
                        target: target.as_ref(),
                        document_key: &document_key,
                        draft_document_origin: draft_document_origin.as_ref(),
                    });
                    match choice {
                        ChannelCredentialChoice::StoredPurpose => return Ok(key),
                        ChannelCredentialChoice::UnreachableOwnCredential => {
                            return Err(stored_credential_origin_mismatch("image"));
                        }
                        ChannelCredentialChoice::Document(choice) => {
                            return self.resolve_inherited_credential(choice).await;
                        }
                    }
                }
                self.inherited_document_credential(
                    request,
                    target.as_ref(),
                    document,
                    stored_document_origin.as_ref(),
                )
                .await
            }
        }
    }

    /// The persisted document origin.
    ///
    /// Only the committed document endpoint can authorize a stored secret; the
    /// caller-supplied document draft is an inheritance source of values, never
    /// of credential origins.
    async fn stored_document_origin(&self) -> Result<Option<Origin>> {
        let stored = self.db.get_runtime_settings().await?;
        Ok(stored
            .as_ref()
            .and_then(|settings| origin_of(&settings.embedding.base_url)))
    }

    /// The inherited document credential, when it applies to this probe.
    async fn inherited_document_credential(
        &self,
        request: &TestRuntimeEmbeddingRequest,
        target: Option<&Origin>,
        document: &DocumentEmbeddingSource,
        stored_document_origin: Option<&Origin>,
    ) -> Result<Option<String>> {
        let document_key = request.document_api_key.clone().unwrap_or_default();
        let draft_document_origin = origin_of(&document.base_url);
        let choice = inherited_credential(
            &document_key,
            target,
            draft_document_origin.as_ref(),
            stored_document_origin,
        );
        self.resolve_inherited_credential(choice).await
    }

    /// Turn the inherited credential choice into a key for this call.
    async fn resolve_inherited_credential(
        &self,
        choice: InheritedCredential,
    ) -> Result<Option<String>> {
        match choice {
            InheritedCredential::Draft(key) => Ok(Some(key)),
            InheritedCredential::Stored => self.embedding_secrets.resolve().await,
            InheritedCredential::None => Ok(None),
        }
    }

    pub async fn test_embedding_connection(
        &self,
        request: &TestRuntimeEmbeddingRequest,
    ) -> Result<()> {
        // A probe never writes. A supplied key is used as given, and otherwise
        // the stored key of the selected purpose is resolved through the store,
        // which fails closed rather than reporting no credential.
        //
        // Only a channel probe inherits from a document configuration. The
        // document probe submits its own complete draft, so the persisted row
        // contributes nothing beyond the credential origin.
        let document = match request.purpose {
            TestRuntimeEmbeddingPurpose::Document => DocumentEmbeddingSource::default(),
            _ => self.probe_document_source(request).await?,
        };
        let effective = self.probe_effective_settings(request, &document)?;
        let api_key = self
            .probe_credential(request, &effective.base_url, &document)
            .await?;
        if request.purpose == TestRuntimeEmbeddingPurpose::Image {
            // The image probe sends one synthetic 1x1 PNG through the image
            // input shape and validates the returned count, dimensions, and
            // finite values. It never writes settings or touches the document
            // provider.
            let provider = crate::image_embedding::build_image_provider(
                &crate::config::ImageEmbeddingConfig {
                    base_url: effective.base_url.trim().to_string(),
                    api_key,
                    model: effective.model.trim().to_string(),
                    query_model: None,
                    dimensions: effective.dimensions as usize,
                    timeout: std::time::Duration::from_secs(effective.timeout_secs),
                    collection_name: "probe".to_string(),
                },
            )?;
            provider.embed_images(&[PROBE_IMAGE_PNG.to_vec()]).await?;
            return Ok(());
        }
        // Construction performs no I/O; the probe embeds one synthetic string.
        // Every error path carries endpoint/model only, never the key.
        let provider =
            crate::embedding::build_probe_provider_with_config(crate::config::EmbeddingConfig {
                base_url: effective.base_url.trim().to_string(),
                api_key: api_key.clone(),
                model: effective.model.trim().to_string(),
                query_model: None,
                dimensions: effective.dimensions as usize,
                timeout: std::time::Duration::from_secs(effective.timeout_secs),
            })?;
        provider.embed_texts(&[PROBE_TEXT.to_string()]).await?;

        // A second, distinct query model is a second provider contract, so it is
        // probed as well: a reachable document model says nothing about the
        // alias that embeds queries.
        let document_model = effective.model.trim();
        let query_model = effective.query_model.clone();
        if let Some(query_model) = query_model
            && query_model.trim() != document_model
        {
            let query_provider = crate::embedding::build_probe_provider_with_config(
                crate::config::EmbeddingConfig {
                    base_url: effective.base_url.trim().to_string(),
                    api_key,
                    model: query_model.trim().to_string(),
                    query_model: None,
                    dimensions: effective.dimensions as usize,
                    timeout: std::time::Duration::from_secs(effective.timeout_secs),
                },
            )?;
            query_provider
                .embed_texts(&[PROBE_TEXT.to_string()])
                .await?;
        }
        Ok(())
    }

    /// The effective values the probe exercises.
    ///
    /// An optional channel resolves its sparse request fields against the
    /// document inheritance source, exactly like its save does. The document
    /// purpose resolves only the submitted draft: it is complete on its own and
    /// never inherits the persisted values.
    fn probe_effective_settings(
        &self,
        request: &TestRuntimeEmbeddingRequest,
        document: &DocumentEmbeddingSource,
    ) -> Result<ProbeSettings> {
        // An explicitly supplied number is a value, never an inheritance: a zero
        // is refused here, before the sparse resolution normalizes anything.
        if request.dimensions == Some(0) {
            return Err(anyhow::anyhow!(
                "the embedding probe dimensions must be greater than 0"
            ));
        }
        if request.timeout_secs == Some(0) {
            return Err(anyhow::anyhow!(
                "the embedding probe timeout_secs must be greater than 0"
            ));
        }
        let resolved = match request.purpose {
            TestRuntimeEmbeddingPurpose::Image => {
                let overrides = image_overrides_from_probe(request);
                resolve_image_settings(&overrides, Some(document)).map(|effective| {
                    (
                        effective.base_url,
                        effective.model,
                        effective.query_model,
                        effective.dimensions,
                        effective.timeout_secs,
                    )
                })
            }
            // The document draft is the whole candidate a save would persist,
            // so it resolves its own submitted values and inherits nothing. A
            // cleared query model therefore stays absent instead of resurrecting
            // the stale persisted alias.
            TestRuntimeEmbeddingPurpose::Document => {
                resolve_document_probe(request).map(|effective| {
                    (
                        effective.base_url,
                        effective.model,
                        effective.query_model,
                        effective.dimensions,
                        effective.timeout_secs,
                    )
                })
            }
            TestRuntimeEmbeddingPurpose::Code => {
                let overrides = code_overrides_from_probe(request);
                resolve_code_settings(&overrides, Some(document)).map(|effective| {
                    (
                        effective.base_url,
                        effective.model,
                        effective.query_model,
                        effective.dimensions,
                        effective.timeout_secs,
                    )
                })
            }
        };
        let (base_url, model, query_model, dimensions, timeout_secs) = resolved.ok_or_else(|| {
            anyhow::anyhow!(
                "the embedding probe has no usable base_url, model, dimensions, and timeout_secs; \
                 the document draft must be complete, and a channel needs a document \
                 configuration to inherit them from"
            )
        })?;
        Ok(ProbeSettings {
            base_url,
            model,
            query_model,
            dimensions: dimensions as usize,
            timeout_secs,
        })
    }
}

/// The effective values one probe exercises.
#[derive(Debug, Clone)]
struct ProbeSettings {
    base_url: String,
    model: String,
    query_model: Option<String>,
    dimensions: usize,
    timeout_secs: u64,
}

/// A visible refusal instead of a substituted credential.
///
/// The stored own-purpose credential exists but belongs to another endpoint than
/// the one under test. Falling back to the document credential would report a
/// success the save does not reproduce, and a stored secret may not travel to an
/// endpoint it was not issued for before that change is committed.
fn stored_credential_origin_mismatch(purpose: &str) -> anyhow::Error {
    DomainError::conflict(format!(
        "the stored {purpose} embedding credential cannot be used for the endpoint under test \
         because it belongs to a different origin; save the endpoint change, or enter a \
         credential for this endpoint"
    ))
    .into()
}
