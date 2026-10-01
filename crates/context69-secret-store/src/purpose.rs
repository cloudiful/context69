//! The purpose and key-name catalogue.
//!
//! A *purpose* names the single owner of one secret, and it is sealed into the
//! ciphertext, so a value written for one owner can never be opened by another.
//! The enum is the whole catalogue: adding a category means adding a variant
//! here, which makes an unowned category impossible to invent at a call site.
//!
//! A *key name* names one secret within its purpose. Singletons use the fixed
//! constants in [`key_names`]; categories that own one secret per record (a
//! source connection, a Git provider connection, a webhook registration) build a
//! bounded name from an existing identifier with [`SecretKeyName::with_prefix`].
//! Both are validated before anything is written, so a malformed dynamic
//! identifier is refused at the call site rather than becoming a stored row.

use std::fmt;

use context69_secret_crypto::frame::{
    MAX_KEY_NAME_LEN, MAX_PURPOSE_LEN, is_valid_key_name, is_valid_purpose,
};

use crate::error::SecretStoreError;

/// Fixed key names, and the prefixes for the categories that own one secret per
/// record.
///
/// A prefix is a namespace, not a complete name: the caller appends an existing
/// record identifier and [`SecretKeyName::with_prefix`] validates the result
/// against the same bound a fixed name has to satisfy.
pub mod key_names {
    /// Browser session cookie signing key. Its value is the historical row key,
    /// so renaming it would orphan every deployed signing key.
    pub const BROWSER_SESSION_SIGNING_KEY: &str = "browser_session_signing_key_v2";
    /// Embedding provider API key (singleton).
    pub const EMBEDDING_API_KEY: &str = "embedding_api_key";
    /// Search / rerank provider API key (singleton).
    pub const SEARCH_API_KEY: &str = "search_api_key";
    /// Docling VLM provider API key (singleton).
    pub const DOCLING_VLM_API_KEY: &str = "docling_vlm_api_key";
    /// Shared translation / extraction provider API key (singleton).
    pub const TRANSLATION_PROVIDER_API_KEY: &str = "translation_provider_api_key";
    /// Runtime S3 secret key (singleton). The access key stays a non-secret
    /// identifier outside reversible storage.
    pub const RUNTIME_S3_SECRET_KEY: &str = "runtime_s3.secret_key";

    /// Per source connection database URL, including embedded credentials.
    pub const SOURCE_CONNECTION_DATABASE_URL_PREFIX: &str = "source_connection.database_url.";
    /// Per Git provider connection personal access token.
    pub const GIT_PROVIDER_TOKEN_PREFIX: &str = "git_provider.token.";
    /// Per Git provider connection GitHub App private key.
    pub const GITHUB_APP_PRIVATE_KEY_PREFIX: &str = "github_app.private_key.";
    /// Per Git webhook registration signing secret.
    pub const GIT_WEBHOOK_SIGNING_SECRET_PREFIX: &str = "git_webhook.signing_secret.";
}

/// The owner of one persisted reversible secret.
///
/// Exhaustive on purpose: every category the store will ever hold is listed
/// here, so an owning feature cannot invent a purpose string at a call site and
/// so the catalogue can be asserted complete in a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretPurpose {
    /// Key material that signs browser session cookies.
    BrowserSessionSigningKey,
    /// Embedding provider API key.
    EmbeddingApiKey,
    /// Search and rerank provider API key.
    SearchApiKey,
    /// Docling VLM provider API key.
    DoclingVlmApiKey,
    /// Provider API key shared by translation and extraction.
    TranslationProviderApiKey,
    /// Source connection database URL, including embedded credentials.
    SourceConnectionDatabaseUrl,
    /// Runtime S3 secret key.
    RuntimeS3SecretKey,
    /// Git provider connection personal access token.
    GitProviderToken,
    /// Git provider connection GitHub App private key.
    GitHubAppPrivateKey,
    /// Git webhook registration signing secret.
    GitWebhookSigningSecret,
}

impl SecretPurpose {
    /// Every purpose this build knows, in a stable order.
    pub const ALL: [SecretPurpose; 10] = [
        Self::BrowserSessionSigningKey,
        Self::EmbeddingApiKey,
        Self::SearchApiKey,
        Self::DoclingVlmApiKey,
        Self::TranslationProviderApiKey,
        Self::SourceConnectionDatabaseUrl,
        Self::RuntimeS3SecretKey,
        Self::GitProviderToken,
        Self::GitHubAppPrivateKey,
        Self::GitWebhookSigningSecret,
    ];

    /// The purpose string sealed into the frame and stored on the row.
    ///
    /// It is also the domain separator, so a value sealed under one purpose
    /// cannot be opened under another.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserSessionSigningKey => "browser_session.signing_key",
            Self::EmbeddingApiKey => "embedding.api_key",
            Self::SearchApiKey => "search.api_key",
            Self::DoclingVlmApiKey => "docling.vlm_api_key",
            Self::TranslationProviderApiKey => "translation.api_key",
            Self::SourceConnectionDatabaseUrl => "source_connection.database_url",
            Self::RuntimeS3SecretKey => "runtime_s3.secret_key",
            Self::GitProviderToken => "git_provider.token",
            Self::GitHubAppPrivateKey => "github_app.private_key",
            Self::GitWebhookSigningSecret => "git_webhook.signing_secret",
        }
    }

    /// The fixed key name for a singleton category.
    ///
    /// Returns `None` for a category that owns one secret per record; those use
    /// [`SecretKeyName::with_prefix`] with the matching prefix.
    pub const fn singleton_key_name(self) -> Option<&'static str> {
        match self {
            Self::BrowserSessionSigningKey => Some(key_names::BROWSER_SESSION_SIGNING_KEY),
            Self::EmbeddingApiKey => Some(key_names::EMBEDDING_API_KEY),
            Self::SearchApiKey => Some(key_names::SEARCH_API_KEY),
            Self::DoclingVlmApiKey => Some(key_names::DOCLING_VLM_API_KEY),
            Self::TranslationProviderApiKey => Some(key_names::TRANSLATION_PROVIDER_API_KEY),
            Self::RuntimeS3SecretKey => Some(key_names::RUNTIME_S3_SECRET_KEY),
            Self::SourceConnectionDatabaseUrl
            | Self::GitProviderToken
            | Self::GitHubAppPrivateKey
            | Self::GitWebhookSigningSecret => None,
        }
    }

    /// The key-name prefix for a category that owns one secret per record.
    pub const fn key_name_prefix(self) -> Option<&'static str> {
        match self {
            Self::SourceConnectionDatabaseUrl => {
                Some(key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX)
            }
            Self::GitProviderToken => Some(key_names::GIT_PROVIDER_TOKEN_PREFIX),
            Self::GitHubAppPrivateKey => Some(key_names::GITHUB_APP_PRIVATE_KEY_PREFIX),
            Self::GitWebhookSigningSecret => Some(key_names::GIT_WEBHOOK_SIGNING_SECRET_PREFIX),
            _ => None,
        }
    }

    /// Whether this category is stored once for the whole deployment.
    pub const fn is_singleton(self) -> bool {
        self.singleton_key_name().is_some()
    }

    /// Resolves a stored purpose string back to a known purpose.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|purpose| purpose.as_str() == name)
    }

    /// Rejects a purpose that is not a bounded identifier.
    ///
    /// Every catalogue entry is a constant that satisfies this, so the helper
    /// exists for the *stored* form, where a row's purpose came from a
    /// different build or a hand edit.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::InvalidPurpose`] when the name is empty, oversized,
    /// or outside the accepted character set.
    pub fn validate_name(name: &str) -> Result<(), SecretStoreError> {
        if is_valid_purpose(name) && Self::parse(name).is_some() {
            Ok(())
        } else {
            Err(SecretStoreError::InvalidPurpose)
        }
    }
}

impl fmt::Display for SecretPurpose {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One secret's name within its purpose, validated at construction.
///
/// A key name is not a secret, but it is a row key and it is authenticated into
/// the ciphertext, so it is held in a type that cannot be built from an
/// unbounded or malformed string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretKeyName(String);

impl SecretKeyName {
    /// Validates a fixed or fully built key name.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::InvalidKeyName`] when the name is empty, longer than
    /// [`MAX_KEY_NAME_LEN`] bytes, or outside the accepted character set.
    pub fn new(value: &str) -> Result<Self, SecretStoreError> {
        if is_valid_key_name(value) {
            Ok(Self(value.to_string()))
        } else {
            Err(SecretStoreError::InvalidKeyName)
        }
    }

    /// Builds the bounded key name for a record-scoped category.
    ///
    /// The record identifier must be non-empty as well as well-formed: an empty
    /// one would collapse every record of that category onto the bare prefix and
    /// silently make them share one secret.
    ///
    /// # Errors
    ///
    /// [`SecretStoreError::InvalidKeyName`] when the record is empty or the
    /// concatenation is not a valid key name. A record identifier that cannot be
    /// embedded — whitespace, a separator, or a length that overflows
    /// [`MAX_KEY_NAME_LEN`] — is refused here, before any storage is touched.
    pub fn with_prefix(prefix: &str, record: &str) -> Result<Self, SecretStoreError> {
        if record.is_empty() {
            return Err(SecretStoreError::InvalidKeyName);
        }
        Self::new(&format!("{prefix}{record}"))
    }

    /// The validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretKeyName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<&str> for SecretKeyName {
    type Error = SecretStoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Longest accepted purpose, re-exported for callers that report their own bound.
pub const LONGEST_PURPOSE: usize = MAX_PURPOSE_LEN;
/// Longest accepted key name, re-exported for callers that report their own bound.
pub const LONGEST_KEY_NAME: usize = MAX_KEY_NAME_LEN;
