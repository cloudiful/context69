use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SourcePolicy {
    #[default]
    Retain,
    ReleaseAfterProcessing,
}

impl SourcePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retain => "retain",
            Self::ReleaseAfterProcessing => "release_after_processing",
        }
    }

    pub fn is_release(self) -> bool {
        matches!(self, Self::ReleaseAfterProcessing)
    }

    pub fn as_delete_flag(self) -> bool {
        self.is_release()
    }
}

impl std::fmt::Display for SourcePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SourcePolicy {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "retain" => Ok(Self::Retain),
            "release_after_processing" => Ok(Self::ReleaseAfterProcessing),
            other => Err(anyhow::anyhow!("unsupported source policy: {other}")),
        }
    }
}

/// v0.18 canonical upload metadata: `metadata_json` is an explicit object
/// map ([`crate::MetadataObject`]).
///
/// Object-ness is enforced at the type boundary: deserializing a scalar or
/// array `metadata_json` fails, and JSON Schema advertises an object.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalUploadMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata_json: context69_contracts_core::common::MetadataObject,
}

impl CanonicalUploadMetadata {
    pub fn is_empty(&self) -> bool {
        self.external_id.is_none()
            && self.source_uri.is_none()
            && self.published_at.is_none()
            && self.metadata_json.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestOptions {
    #[serde(default)]
    pub metadata: CanonicalUploadMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<context69_contracts_translation::TranslationDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<context69_contracts_extraction::ExtractionDirective>,
    #[serde(default)]
    pub source_policy: SourcePolicy,
}

impl IngestOptions {
    pub fn retain() -> Self {
        Self {
            source_policy: SourcePolicy::Retain,
            ..Self::default()
        }
    }

    pub fn is_release(&self) -> bool {
        self.source_policy.is_release()
    }

    pub fn as_delete_flag(&self) -> bool {
        self.source_policy.as_delete_flag()
    }
}
