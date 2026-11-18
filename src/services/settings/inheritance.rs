//! The effective configuration of the optional embedding channels.
//!
//! The code and image channels store sparse overrides: an absent field means
//! "inherit the document embedding value". This module is the one place that
//! turns those overrides plus the document configuration into the values a
//! provider actually runs with, so the save, the startup hydration, the
//! deferred refresh, and the probe can never drift apart.
//!
//! Two rules shape the resolution:
//!
//! * a field inherits independently, and the query model has its own order —
//!   an explicit channel query model wins, then the document query model, and
//!   finally the effective channel model;
//! * a credential never leaves its origin — the channel's own credential is
//!   used first, and the document credential is used only when the effective
//!   endpoint is the document origin.

use crate::contracts::settings::{SecretPatch, UpdateRuntimeEmbeddingSettings};
use crate::db::{
    StoredRuntimeCodeEmbeddingSettings, StoredRuntimeEmbeddingSettings,
    StoredRuntimeImageEmbeddingSettings,
};
use crate::support::normalize::normalize_optional_string;

/// The default physical collection image vectors live in.
///
/// An image channel that inherits its collection name resolves to this
/// collection, so an enabled image channel always has a usable destination that
/// is neither the document nor the code collection.
pub const DEFAULT_IMAGE_COLLECTION_NAME: &str =
    crate::contracts::settings::DEFAULT_IMAGE_COLLECTION_NAME;

/// The document embedding configuration an optional channel inherits from.
#[derive(Debug, Clone, Default)]
pub struct DocumentEmbeddingSource {
    pub base_url: String,
    pub model: String,
    /// The model queries are embedded with. Absent means the document model
    /// embeds queries too.
    pub query_model: Option<String>,
    pub dimensions: u64,
    pub timeout_secs: u64,
    /// The document credential resolved for this process. It is an in-memory
    /// value, never a persisted one.
    pub api_key: Option<String>,
}

impl DocumentEmbeddingSource {
    /// The source the stored document settings describe.
    pub fn from_stored(stored: &StoredRuntimeEmbeddingSettings) -> Self {
        Self {
            base_url: stored.base_url.trim().to_string(),
            model: stored.model.trim().to_string(),
            query_model: stored
                .query_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string),
            dimensions: stored.dimensions as u64,
            timeout_secs: stored.timeout_secs,
            api_key: stored.api_key.clone(),
        }
    }

    /// The source the document block of the request describes.
    ///
    /// A save resolves against the candidate of the same request, never against
    /// the stale persisted row, so the channel and the document it inherits
    /// from are always the ones the save installs together.
    pub fn from_update(request: &UpdateRuntimeEmbeddingSettings, api_key: Option<String>) -> Self {
        Self {
            base_url: request.base_url.trim().to_string(),
            model: request.model.trim().to_string(),
            query_model: request
                .query_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string),
            dimensions: request.dimensions as u64,
            timeout_secs: request.timeout_secs,
            api_key,
        }
    }
}

/// The values one optional channel actually runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveCodeSettings {
    pub base_url: String,
    pub model: String,
    /// The model queries are embedded with.
    pub query_model: Option<String>,
    pub dimensions: u64,
    pub timeout_secs: u64,
}

/// The values the image channel actually runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveImageSettings {
    pub base_url: String,
    pub model: String,
    /// The model queries are embedded with.
    pub query_model: Option<String>,
    pub dimensions: u64,
    pub timeout_secs: u64,
    /// The independent physical collection image vectors live in.
    pub collection_name: String,
}

/// The credential one optional channel runs with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelCredential {
    /// The credential in effect.
    pub api_key: Option<String>,
    /// Whether that credential is the inherited document one.
    pub inherits_document: bool,
}

/// The origin of one endpoint: scheme, host, and effective port.
///
/// Only these three parts decide whether a credential may travel between two
/// endpoints, so a path prefix, a query, or a trailing slash never changes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

/// The canonical origin of one endpoint.
///
/// The URL is parsed with the established URL parser, so the scheme and the
/// host are normalized the way a provider sees them and the default port of a
/// scheme is the effective one.
pub fn origin_of(base_url: &str) -> Option<Origin> {
    let parsed = reqwest::Url::parse(base_url.trim()).ok()?;
    let scheme = parsed.scheme().to_ascii_lowercase();
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed.port_or_known_default()?;
    Some(Origin { scheme, host, port })
}

/// Whether two endpoints share one origin.
pub fn same_origin(left: &str, right: &str) -> bool {
    match (origin_of(left), origin_of(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// The effective code configuration of one stored override set.
///
/// The fields inherit independently. The query model has its own order: an
/// explicit channel query model wins, then the document query model, and
/// finally the effective channel model — so an explicit channel model without
/// an explicit document query model embeds queries with that model.
///
/// `None` means the configuration cannot be completed: an enabled channel
/// needs a document configuration to inherit from, or overrides of its own.
pub fn resolve_code_settings(
    overrides: &StoredRuntimeCodeEmbeddingSettings,
    document: Option<&DocumentEmbeddingSource>,
) -> Option<EffectiveCodeSettings> {
    let base_url = resolve_text(
        overrides.base_url.as_deref(),
        document.map(|document| document.base_url.as_str()),
    )?;
    let model = resolve_text(
        overrides.model.as_deref(),
        document.map(|document| document.model.as_str()),
    )?;
    let dimensions = overrides
        .dimensions
        .filter(|dimensions| *dimensions > 0)
        .or_else(|| document.map(|document| document.dimensions))?;
    let timeout_secs = overrides
        .timeout_secs
        .filter(|timeout| *timeout > 0)
        .or_else(|| document.map(|document| document.timeout_secs))?;
    let query_model = resolve_text(
        overrides.query_model.as_deref(),
        document.and_then(|document| document.query_model.as_deref()),
    )
    .unwrap_or_else(|| model.clone());
    Some(EffectiveCodeSettings {
        base_url,
        model,
        query_model: Some(query_model),
        dimensions,
        timeout_secs,
    })
}

/// The effective image configuration of one stored override set.
///
/// The image channel inherits exactly like the code channel, and its collection
/// name resolves to the default image collection when it is absent.
pub fn resolve_image_settings(
    overrides: &StoredRuntimeImageEmbeddingSettings,
    document: Option<&DocumentEmbeddingSource>,
) -> Option<EffectiveImageSettings> {
    let base_url = resolve_text(
        overrides.base_url.as_deref(),
        document.map(|document| document.base_url.as_str()),
    )?;
    let model = resolve_text(
        overrides.model.as_deref(),
        document.map(|document| document.model.as_str()),
    )?;
    let dimensions = overrides
        .dimensions
        .filter(|dimensions| *dimensions > 0)
        .or_else(|| document.map(|document| document.dimensions))?;
    let timeout_secs = overrides
        .timeout_secs
        .filter(|timeout| *timeout > 0)
        .or_else(|| document.map(|document| document.timeout_secs))?;
    let query_model = resolve_text(
        overrides.query_model.as_deref(),
        document.and_then(|document| document.query_model.as_deref()),
    )
    .unwrap_or_else(|| model.clone());
    let collection_name = override_text(overrides.collection_name.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_IMAGE_COLLECTION_NAME.to_string());
    Some(EffectiveImageSettings {
        base_url,
        model,
        query_model: Some(query_model),
        dimensions,
        timeout_secs,
        collection_name,
    })
}

/// The values one document probe actually exercises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveDocumentSettings {
    pub base_url: String,
    pub model: String,
    /// The model queries are embedded with. Absent means the document model
    /// embeds queries too, exactly like the stored row after a blank clear.
    pub query_model: Option<String>,
    pub dimensions: u64,
    pub timeout_secs: u64,
}

/// The effective configuration of the unsaved document draft a probe submits.
///
/// The document block is complete on its own, so the submitted request is the
/// whole candidate a save would persist: nothing inherits, and in particular a
/// cleared query model stays absent instead of resolving a stale persisted
/// alias. The persisted row contributes only the credential origin of the
/// probe, never a value under test.
///
/// `None` means the draft is not a complete document configuration; a save of
/// the same draft could not run with it either.
pub fn resolve_document_probe(
    request: &crate::contracts::settings::TestRuntimeEmbeddingRequest,
) -> Option<EffectiveDocumentSettings> {
    Some(EffectiveDocumentSettings {
        base_url: probe_text(request.base_url.as_deref())?,
        model: probe_text(request.model.as_deref())?,
        dimensions: request
            .dimensions
            .filter(|dimensions| *dimensions > 0)
            .map(|dimensions| dimensions as u64)?,
        timeout_secs: request.timeout_secs.filter(|timeout| *timeout > 0)?,
        query_model: probe_text(request.query_model.as_deref()),
    })
}

/// The key a probe may use for its own purpose.
///
/// The key intent owns the decision, exactly as it owns the save: a pending
/// clear (`Inherited`) tests the credential that is in effect after the clear,
/// so a replacement key the same draft still carries is discarded. Only the
/// `Stored` intent uses a supplied key, and a blank one is absent.
pub fn submitted_probe_key(
    intent: crate::contracts::settings::TestRuntimeEmbeddingKeyIntent,
    supplied: Option<&str>,
) -> Option<String> {
    if intent != crate::contracts::settings::TestRuntimeEmbeddingKeyIntent::Stored {
        return None;
    }
    normalize_optional_string(supplied.map(str::to_string))
}

/// The credential one optional channel runs with.
///
/// The channel's own credential wins. The document credential is used only when
/// the effective endpoint is the document origin, so a stored credential never
/// travels to an endpoint it was not issued for. An endpoint without a
/// credential stays usable: anonymous providers are supported.
pub fn channel_credential(
    own_key: Option<String>,
    effective_base_url: &str,
    document: Option<&DocumentEmbeddingSource>,
) -> ChannelCredential {
    if let Some(own_key) = own_key {
        return ChannelCredential {
            api_key: Some(own_key),
            inherits_document: false,
        };
    }
    let Some(document) = document else {
        return ChannelCredential::default();
    };
    if !same_origin(effective_base_url, &document.base_url) {
        return ChannelCredential::default();
    }
    match document.api_key.clone() {
        Some(api_key) => ChannelCredential {
            api_key: Some(api_key),
            inherits_document: true,
        },
        None => ChannelCredential::default(),
    }
}

/// Whether the credential in effect is the inherited document credential.
///
/// The channel's own credential wins, so an own credential never reports an
/// inheritance. Without one, the document credential is in effect exactly when
/// the effective endpoint is the document origin and the document credential
/// exists.
pub fn inherits_document_credential(
    has_own_key: bool,
    has_document_key: bool,
    effective_base_url: &str,
    document_base_url: &str,
) -> bool {
    !has_own_key && has_document_key && same_origin(effective_base_url, document_base_url)
}

/// Whether a probe may use the stored own-purpose credential at its target.
///
/// A stored secret only travels to the endpoint it was configured for. A
/// channel that stores its own endpoint has that endpoint as its origin; a
/// channel that inherits its endpoint was configured for the persisted document
/// endpoint, so that is the only origin the stored credential may reach. A
/// caller-supplied document draft never widens this boundary.
pub fn stored_purpose_credential_applies(
    stored_channel_endpoint: Option<&str>,
    stored_document_origin: Option<&Origin>,
    target: Option<&Origin>,
) -> bool {
    let Some(target) = target else {
        return false;
    };
    let origin = match stored_channel_endpoint {
        Some(base_url) => origin_of(base_url),
        None => stored_document_origin.cloned(),
    };
    origin.as_ref() == Some(target)
}

/// The document credential a probe inherits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritedCredential {
    /// The credential the operator entered in the unsaved document draft.
    Draft(String),
    /// The stored document credential, used at its persisted origin.
    Stored,
    /// No credential applies.
    None,
}

/// Resolve the document credential a probe inherits.
///
/// Only the persisted document origin can authorize the stored document
/// credential: a caller-supplied document draft decides where a *submitted*
/// key travels, never where a stored secret travels. The draft endpoint is the
/// candidate a save would install, so the stored credential applies only when
/// the endpoint under test is the persisted *and* the candidate document origin
/// — otherwise a save would resolve no inherited credential for that channel. A
/// submitted draft key is used only at the document draft origin, which is
/// exactly the endpoint the same save would install it for; a cleared or blank
/// draft credential leaves no credential in effect.
pub fn inherited_credential(
    document_key: &SecretPatch,
    target: Option<&Origin>,
    draft_document_origin: Option<&Origin>,
    stored_document_origin: Option<&Origin>,
) -> InheritedCredential {
    match document_key {
        SecretPatch::Clear => InheritedCredential::None,
        SecretPatch::Set(value) => match normalize_optional_string(Some(value.clone())) {
            Some(key) if draft_document_origin.is_some() && draft_document_origin == target => {
                InheritedCredential::Draft(key)
            }
            _ => InheritedCredential::None,
        },
        SecretPatch::Keep => {
            let stored_applies =
                stored_document_origin.is_some() && stored_document_origin == target;
            let candidate_applies =
                draft_document_origin.is_some() && draft_document_origin == target;
            if !stored_applies || !candidate_applies {
                return InheritedCredential::None;
            }
            InheritedCredential::Stored
        }
    }
}

/// Where a channel probe takes its credential from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelCredentialChoice {
    /// The stored own-purpose credential, at the origin it was configured for.
    StoredPurpose,
    /// A stored own-purpose credential exists but cannot reach the endpoint
    /// under test.
    ///
    /// No other credential may stand in for it: the save keeps the own key and
    /// installs it at the candidate endpoint, so the probe reports the mismatch
    /// instead of succeeding with a credential that save never installs.
    UnreachableOwnCredential,
    /// The inherited document credential.
    Document(InheritedCredential),
}

/// The inputs one channel credential decision depends on.
pub struct ChannelCredentialInput<'a> {
    /// Whether the selected purpose stores a credential of its own.
    pub own_key_present: bool,
    /// The stored channel endpoint override, if the channel stores one.
    pub stored_channel_endpoint: Option<&'a str>,
    /// The origin of the persisted document endpoint.
    pub stored_document_origin: Option<&'a Origin>,
    /// The origin the probe actually exercises.
    pub target: Option<&'a Origin>,
    /// The document credential intent of the unsaved document draft.
    pub document_key: &'a SecretPatch,
    /// The origin of the unsaved document draft.
    pub draft_document_origin: Option<&'a Origin>,
}

/// Resolve which credential a channel probe uses.
///
/// The stored own-purpose credential has precedence, exactly as it does on
/// save. When it exists but its configured origin is not the endpoint under
/// test, the probe reports the mismatch: falling back to the document credential
/// would report success for a credential the save never installs.
pub fn channel_credential_choice(input: ChannelCredentialInput<'_>) -> ChannelCredentialChoice {
    if input.own_key_present {
        if stored_purpose_credential_applies(
            input.stored_channel_endpoint,
            input.stored_document_origin,
            input.target,
        ) {
            return ChannelCredentialChoice::StoredPurpose;
        }
        return ChannelCredentialChoice::UnreachableOwnCredential;
    }
    ChannelCredentialChoice::Document(inherited_credential(
        input.document_key,
        input.target,
        input.draft_document_origin,
        input.stored_document_origin,
    ))
}

/// The code overrides a probe carries.
///
/// The probe resolves the same sparse fields a save stores, so the values it
/// exercises are the ones a save would install.
pub fn code_overrides_from_probe(
    request: &crate::contracts::settings::TestRuntimeEmbeddingRequest,
) -> StoredRuntimeCodeEmbeddingSettings {
    StoredRuntimeCodeEmbeddingSettings {
        enabled: true,
        base_url: probe_text(request.base_url.as_deref()),
        model: probe_text(request.model.as_deref()),
        query_model: probe_text(request.query_model.as_deref()),
        dimensions: request.dimensions.map(|dimensions| dimensions as u64),
        timeout_secs: request.timeout_secs,
        api_key: None,
    }
}

/// The image overrides a probe carries.
pub fn image_overrides_from_probe(
    request: &crate::contracts::settings::TestRuntimeEmbeddingRequest,
) -> StoredRuntimeImageEmbeddingSettings {
    StoredRuntimeImageEmbeddingSettings {
        enabled: true,
        base_url: probe_text(request.base_url.as_deref()),
        collection_name: None,
        model: probe_text(request.model.as_deref()),
        query_model: probe_text(request.query_model.as_deref()),
        dimensions: request.dimensions.map(|dimensions| dimensions as u64),
        timeout_secs: request.timeout_secs,
        api_key: None,
    }
}

/// A probe value that is present.
fn probe_text(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// A stored override value that is present.
///
/// A blank value is absent: it inherits rather than overriding with nothing.
fn override_text(value: Option<&str>) -> Option<&str> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// The value one field resolves to: the override when it is present, and the
/// document value otherwise.
fn resolve_text(override_value: Option<&str>, document_value: Option<&str>) -> Option<String> {
    override_text(override_value)
        .or_else(|| override_text(document_value))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelCredentialChoice, ChannelCredentialInput, DocumentEmbeddingSource,
        InheritedCredential, channel_credential, channel_credential_choice, inherited_credential,
        origin_of, resolve_code_settings, resolve_document_probe, resolve_image_settings,
        same_origin, stored_purpose_credential_applies, submitted_probe_key,
    };
    use crate::contracts::settings::SecretPatch;
    use crate::contracts::settings::{
        TestRuntimeEmbeddingKeyIntent, TestRuntimeEmbeddingPurpose, TestRuntimeEmbeddingRequest,
    };
    use crate::db::{StoredRuntimeCodeEmbeddingSettings, StoredRuntimeImageEmbeddingSettings};

    fn document_probe() -> TestRuntimeEmbeddingRequest {
        TestRuntimeEmbeddingRequest {
            purpose: TestRuntimeEmbeddingPurpose::Document,
            base_url: Some("https://documents.invalid/v2".to_string()),
            model: Some("new-document-model".to_string()),
            query_model: None,
            dimensions: Some(768),
            timeout_secs: Some(30),
            api_key: None,
            key_intent: None,
            document_api_key: None,
            document_base_url: None,
            document_model: None,
            document_query_model: None,
            document_dimensions: None,
            document_timeout_secs: None,
        }
    }

    fn document() -> DocumentEmbeddingSource {
        DocumentEmbeddingSource {
            base_url: "https://documents.invalid/v1".to_string(),
            model: "document-model".to_string(),
            query_model: None,
            dimensions: 768,
            timeout_secs: 30,
            api_key: Some("document-key".to_string()),
        }
    }

    fn code() -> StoredRuntimeCodeEmbeddingSettings {
        StoredRuntimeCodeEmbeddingSettings {
            enabled: true,
            ..Default::default()
        }
    }

    fn image() -> StoredRuntimeImageEmbeddingSettings {
        StoredRuntimeImageEmbeddingSettings {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn an_absent_override_inherits_the_document_value() {
        let effective = resolve_code_settings(&code(), Some(&document()))
            .expect("an empty override set inherits");
        assert_eq!(effective.base_url, "https://documents.invalid/v1");
        assert_eq!(effective.model, "document-model");
        assert_eq!(effective.query_model.as_deref(), Some("document-model"));
        assert_eq!(effective.dimensions, 768);
        assert_eq!(effective.timeout_secs, 30);
    }

    #[test]
    fn an_explicit_override_survives_the_resolution() {
        let mut overrides = code();
        overrides.base_url = Some("https://code.invalid/v1".to_string());
        overrides.model = Some("code-model".to_string());
        overrides.query_model = Some("code-query".to_string());
        overrides.dimensions = Some(1024);
        overrides.timeout_secs = Some(90);
        let effective = resolve_code_settings(&overrides, Some(&document()))
            .expect("a complete override set resolves");
        assert_eq!(effective.base_url, "https://code.invalid/v1");
        assert_eq!(effective.model, "code-model");
        assert_eq!(effective.query_model.as_deref(), Some("code-query"));
        assert_eq!(effective.dimensions, 1024);
        assert_eq!(effective.timeout_secs, 90);
    }

    #[test]
    fn the_fields_inherit_independently() {
        let mut overrides = code();
        overrides.model = Some("code-model".to_string());
        let effective = resolve_code_settings(&overrides, Some(&document()))
            .expect("a partial override set resolves");
        assert_eq!(effective.model, "code-model");
        assert_eq!(effective.base_url, "https://documents.invalid/v1");
        assert_eq!(effective.dimensions, 768);
    }

    #[test]
    fn the_document_query_model_is_honored_and_the_channel_model_is_the_fallback() {
        let mut source = document();
        source.query_model = Some("document-query".to_string());
        let effective =
            resolve_code_settings(&code(), Some(&source)).expect("an empty override set inherits");
        assert_eq!(effective.query_model.as_deref(), Some("document-query"));

        let mut overrides = code();
        overrides.model = Some("code-model".to_string());
        let effective = resolve_code_settings(&overrides, Some(&source))
            .expect("a partial override set resolves");
        assert_eq!(
            effective.query_model.as_deref(),
            Some("document-query"),
            "the document query model stays in effect"
        );

        let mut source = document();
        source.query_model = None;
        let effective = resolve_code_settings(&overrides, Some(&source))
            .expect("a partial override set resolves");
        assert_eq!(
            effective.query_model.as_deref(),
            Some("code-model"),
            "without a document query model the effective channel model embeds queries"
        );
    }

    #[test]
    fn an_enabled_channel_without_a_document_configuration_cannot_be_resolved() {
        assert!(resolve_code_settings(&code(), None).is_none());
        assert!(resolve_image_settings(&image(), None).is_none());
    }

    #[test]
    fn an_absent_image_collection_resolves_to_the_default_namespace() {
        let effective = resolve_image_settings(&image(), Some(&document()))
            .expect("an empty override set inherits");
        assert_eq!(effective.collection_name, "context69_images");

        let mut overrides = image();
        overrides.collection_name = Some("custom_images".to_string());
        let effective = resolve_image_settings(&overrides, Some(&document()))
            .expect("a complete override set resolves");
        assert_eq!(effective.collection_name, "custom_images");
    }

    #[test]
    fn the_own_credential_wins_over_the_document_one() {
        let credential = channel_credential(
            Some("own-key".to_string()),
            "https://documents.invalid/v1",
            Some(&document()),
        );
        assert_eq!(credential.api_key.as_deref(), Some("own-key"));
        assert!(!credential.inherits_document);
    }

    #[test]
    fn the_document_credential_is_inherited_at_the_same_origin_only() {
        let same = channel_credential(None, "https://documents.invalid/v1", Some(&document()));
        assert_eq!(same.api_key.as_deref(), Some("document-key"));
        assert!(same.inherits_document);

        let other = channel_credential(None, "https://other.invalid/v1", Some(&document()));
        assert_eq!(other.api_key, None);
        assert!(!other.inherits_document);
    }

    #[test]
    fn a_channel_without_any_credential_stays_usable() {
        let mut source = document();
        source.api_key = None;
        let credential = channel_credential(None, "https://documents.invalid/v1", Some(&source));
        assert_eq!(credential.api_key, None);
        assert!(!credential.inherits_document);
    }

    #[test]
    fn the_origin_comparison_uses_the_scheme_the_host_and_the_effective_port() {
        assert!(same_origin(
            "https://a.invalid/v1",
            "https://a.invalid/v1/embeddings"
        ));
        assert!(same_origin("https://a.invalid", "https://a.invalid:443/v1"));
        assert!(same_origin("http://a.invalid:80/v1", "http://a.invalid/v1"));
        assert!(!same_origin("http://a.invalid/v1", "https://a.invalid/v1"));
        assert!(!same_origin("https://a.invalid/v1", "https://b.invalid/v1"));
        assert!(!same_origin(
            "https://a.invalid:8443/v1",
            "https://a.invalid/v1"
        ));
        assert!(!same_origin("not a url", "https://a.invalid/v1"));
    }

    #[test]
    fn a_document_change_re_resolves_an_inherited_channel() {
        let before = resolve_code_settings(&code(), Some(&document()))
            .expect("an empty override set inherits");
        let mut changed = document();
        changed.model = "new-document-model".to_string();
        changed.dimensions = 1024;
        let after =
            resolve_code_settings(&code(), Some(&changed)).expect("an empty override set inherits");
        assert_eq!(before.model, "document-model");
        assert_eq!(after.model, "new-document-model");
        assert_eq!(after.dimensions, 1024);
        assert_ne!(
            before.model, after.model,
            "an inherited channel follows the document it inherits from"
        );
    }

    #[test]
    fn a_document_change_leaves_an_explicit_override_alone() {
        let mut overrides = code();
        overrides.model = Some("code-model".to_string());
        let before = resolve_code_settings(&overrides, Some(&document()))
            .expect("a partial override set resolves");
        let mut changed = document();
        changed.model = "new-document-model".to_string();
        let after = resolve_code_settings(&overrides, Some(&changed))
            .expect("a partial override set resolves");
        assert_eq!(before.model, "code-model");
        assert_eq!(after.model, "code-model");
    }

    #[test]
    fn the_probe_resolves_the_same_values_a_save_would_store() {
        let request = crate::contracts::settings::TestRuntimeEmbeddingRequest {
            purpose: crate::contracts::settings::TestRuntimeEmbeddingPurpose::Code,
            base_url: Some("https://code.invalid/v1".to_string()),
            model: Some("code-model".to_string()),
            query_model: None,
            dimensions: Some(1024),
            timeout_secs: Some(30),
            api_key: None,
            key_intent: None,
            document_api_key: None,
            document_base_url: Some("https://documents.invalid/v1".to_string()),
            document_model: Some("document-model".to_string()),
            document_query_model: None,
            document_dimensions: Some(768),
            document_timeout_secs: Some(30),
        };
        let probe_overrides = super::code_overrides_from_probe(&request);
        // The save path builds its stored overrides from the update block, so
        // the parity is between the two projections of the same draft.
        let saved_overrides = super::super::runtime_mappers::stored_code_embedding_from_request(
            &crate::contracts::settings::UpdateCodeEmbeddingSettings {
                enabled: true,
                base_url: request.base_url.clone(),
                model: request.model.clone(),
                query_model: request.query_model.clone(),
                dimensions: request.dimensions,
                timeout_secs: request.timeout_secs,
                api_key: crate::contracts::SecretPatch::Keep,
            },
        );
        assert_eq!(probe_overrides.base_url, saved_overrides.base_url);
        assert_eq!(probe_overrides.model, saved_overrides.model);
        assert_eq!(probe_overrides.query_model, saved_overrides.query_model);
        assert_eq!(probe_overrides.dimensions, saved_overrides.dimensions);
        assert_eq!(probe_overrides.timeout_secs, saved_overrides.timeout_secs);

        let source = DocumentEmbeddingSource {
            base_url: "https://documents.invalid/v1".to_string(),
            model: "document-model".to_string(),
            query_model: None,
            dimensions: 768,
            timeout_secs: 30,
            api_key: None,
        };
        assert_eq!(
            resolve_code_settings(&probe_overrides, Some(&source)),
            resolve_code_settings(&saved_overrides, Some(&source)),
            "the probe and the save resolve the same effective configuration"
        );
    }

    fn origin(base_url: &str) -> super::Origin {
        origin_of(base_url).expect("the fixture endpoint parses")
    }

    #[test]
    fn a_stored_own_key_only_reaches_the_endpoint_it_was_configured_for() {
        let stored_document = origin("https://documents.invalid/v1");
        assert!(
            stored_purpose_credential_applies(
                Some("https://code.invalid/v1"),
                Some(&stored_document),
                Some(&origin("https://code.invalid/v1/v2")),
            ),
            "the same origin keeps the stored own-purpose credential"
        );
        assert!(
            !stored_purpose_credential_applies(
                Some("https://code.invalid/v1"),
                Some(&stored_document),
                Some(&origin("https://documents.invalid/v1")),
            ),
            "an own-purpose credential never moves to another origin"
        );
    }

    #[test]
    fn a_sparse_channel_endpoint_binds_its_own_key_to_the_persisted_document_origin() {
        let stored_document = origin("https://documents.invalid/v1");
        assert!(
            stored_purpose_credential_applies(
                None,
                Some(&stored_document),
                Some(&origin("https://documents.invalid/v2")),
            ),
            "a channel that inherits its endpoint was configured for the document origin"
        );
        assert!(!stored_purpose_credential_applies(
            None,
            Some(&stored_document),
            Some(&origin("https://elsewhere.invalid/v1")),
        ));
        assert!(
            !stored_purpose_credential_applies(
                None,
                None,
                Some(&origin("https://documents.invalid/v1")),
            ),
            "without a persisted document origin no origin authorizes the key"
        );
        assert!(!stored_purpose_credential_applies(
            None,
            Some(&stored_document),
            None
        ));
    }

    #[test]
    fn a_draft_document_origin_never_authorizes_the_stored_document_credential() {
        let stored_document = origin("https://documents.invalid/v1");
        let unrelated_draft = origin("https://gateway.invalid/v1");
        assert_eq!(
            inherited_credential(
                &SecretPatch::Keep,
                Some(&unrelated_draft),
                Some(&unrelated_draft),
                Some(&stored_document),
            ),
            InheritedCredential::None,
            "a caller-supplied document draft must not move a stored secret"
        );
        assert_eq!(
            inherited_credential(
                &SecretPatch::Keep,
                Some(&stored_document),
                Some(&stored_document),
                Some(&stored_document),
            ),
            InheritedCredential::Stored
        );
        assert_eq!(
            inherited_credential(
                &SecretPatch::Keep,
                Some(&origin("https://documents.invalid/v1")),
                None,
                None,
            ),
            InheritedCredential::None,
            "without a persisted document origin no stored secret applies"
        );
    }

    #[test]
    fn a_submitted_document_key_is_used_at_the_draft_origin_only() {
        let draft = origin("https://documents.invalid/v1");
        let elsewhere = origin("https://gateway.invalid/v1");
        assert_eq!(
            inherited_credential(
                &SecretPatch::Set("draft-key".to_string()),
                Some(&draft),
                Some(&draft),
                None,
            ),
            InheritedCredential::Draft("draft-key".to_string()),
            "the unsaved document key is the credential the same save would install"
        );
        assert_eq!(
            inherited_credential(
                &SecretPatch::Set("draft-key".to_string()),
                Some(&elsewhere),
                Some(&draft),
                Some(&draft),
            ),
            InheritedCredential::None
        );
        assert_eq!(
            inherited_credential(
                &SecretPatch::Set("  ".to_string()),
                Some(&draft),
                Some(&draft),
                Some(&draft),
            ),
            InheritedCredential::None,
            "a blank replacement is not a credential"
        );
    }

    #[test]
    fn a_cleared_document_draft_leaves_no_inherited_credential() {
        let draft = origin("https://documents.invalid/v1");
        assert_eq!(
            inherited_credential(
                &SecretPatch::Clear,
                Some(&draft),
                Some(&draft),
                Some(&draft),
            ),
            InheritedCredential::None
        );
    }

    #[test]
    fn keep_requires_the_candidate_document_origin() {
        let persisted = origin("https://documents.invalid/v1");
        let candidate = origin("https://gateway.invalid/v1");
        assert_eq!(
            inherited_credential(
                &SecretPatch::Keep,
                Some(&persisted),
                Some(&candidate),
                Some(&persisted),
            ),
            InheritedCredential::None,
            "a save resolves no inherited credential when the candidate document origin moved"
        );
        assert_eq!(
            inherited_credential(
                &SecretPatch::Keep,
                Some(&persisted),
                Some(&persisted),
                Some(&candidate),
            ),
            InheritedCredential::None,
            "a draft that kept a previous origin does not authorize the stored secret either"
        );
    }

    #[test]
    fn an_own_credential_at_its_origin_wins_over_the_document_credential() {
        let stored_document = origin("https://documents.invalid/v1");
        let channel = origin("https://code.invalid/v1");
        assert_eq!(
            channel_credential_choice(ChannelCredentialInput {
                own_key_present: true,
                stored_channel_endpoint: Some("https://code.invalid/v1"),
                stored_document_origin: Some(&stored_document),
                target: Some(&channel),
                document_key: &SecretPatch::Set("document-draft-key".to_string()),
                draft_document_origin: Some(&stored_document),
            }),
            ChannelCredentialChoice::StoredPurpose
        );
    }

    #[test]
    fn an_unreachable_own_credential_never_falls_back_to_the_document_credential() {
        let stored_document = origin("https://documents.invalid/v1");
        let candidate = origin("https://gateway.invalid/v1");
        assert_eq!(
            channel_credential_choice(ChannelCredentialInput {
                own_key_present: true,
                stored_channel_endpoint: Some("https://documents.invalid/v1"),
                stored_document_origin: Some(&stored_document),
                target: Some(&candidate),
                document_key: &SecretPatch::Set("document-draft-key".to_string()),
                draft_document_origin: Some(&candidate),
            }),
            ChannelCredentialChoice::UnreachableOwnCredential,
            "the save keeps the own key, so the probe must not succeed with another credential"
        );
    }

    #[test]
    fn a_channel_without_an_own_credential_inherits_the_document_credential() {
        let stored_document = origin("https://documents.invalid/v1");
        assert_eq!(
            channel_credential_choice(ChannelCredentialInput {
                own_key_present: false,
                stored_channel_endpoint: None,
                stored_document_origin: Some(&stored_document),
                target: Some(&stored_document),
                document_key: &SecretPatch::Set("document-draft-key".to_string()),
                draft_document_origin: Some(&stored_document),
            }),
            ChannelCredentialChoice::Document(InheritedCredential::Draft(
                "document-draft-key".to_string()
            ))
        );
    }

    #[test]
    fn a_document_probe_never_resurrects_a_cleared_query_alias() {
        let effective =
            resolve_document_probe(&document_probe()).expect("a complete document draft resolves");
        assert_eq!(effective.base_url, "https://documents.invalid/v2");
        assert_eq!(effective.model, "new-document-model");
        assert_eq!(
            effective.query_model, None,
            "a cleared draft query model is no alias, not the stale persisted one"
        );
        assert_eq!(effective.dimensions, 768);
        assert_eq!(effective.timeout_secs, 30);
    }

    #[test]
    fn a_document_probe_keeps_the_submitted_query_alias() {
        let mut request = document_probe();
        request.query_model = Some("  fresh-query-model  ".to_string());
        let effective =
            resolve_document_probe(&request).expect("a complete document draft resolves");
        assert_eq!(effective.query_model.as_deref(), Some("fresh-query-model"));
    }

    #[test]
    fn an_incomplete_document_draft_has_no_effective_configuration() {
        let mut request = document_probe();
        request.base_url = Some("   ".to_string());
        assert_eq!(
            resolve_document_probe(&request),
            None,
            "a blank endpoint is not a document configuration"
        );
        let mut request = document_probe();
        request.dimensions = Some(0);
        assert_eq!(
            resolve_document_probe(&request),
            None,
            "an explicit zero is not a usable dimension"
        );
    }

    #[test]
    fn a_pending_clear_ignores_a_replacement_key_the_draft_still_carries() {
        assert_eq!(
            submitted_probe_key(TestRuntimeEmbeddingKeyIntent::Inherited, Some("typed-key")),
            None,
            "a clear discards the typed replacement exactly as the save does"
        );
        assert_eq!(
            submitted_probe_key(TestRuntimeEmbeddingKeyIntent::Stored, Some(" typed-key ")),
            Some("typed-key".to_string())
        );
        assert_eq!(
            submitted_probe_key(TestRuntimeEmbeddingKeyIntent::Stored, Some("   ")),
            None,
            "a blank supplied key is not a credential"
        );
    }
}
