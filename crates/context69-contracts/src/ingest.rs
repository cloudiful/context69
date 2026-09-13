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

    pub fn from_delete_flag(delete_source_after_processing: bool) -> Self {
        if delete_source_after_processing {
            Self::ReleaseAfterProcessing
        } else {
            Self::Retain
        }
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

/// v0.16 canonical upload metadata: same wire keys as the v0.15
/// [`crate::LibraryFileUploadMetadata`], but `metadata_json` is an explicit
/// object map ([`crate::MetadataObject`]) instead of `serde_json::Value`.
///
/// Object-ness is enforced at the type boundary: deserializing a scalar or
/// array `metadata_json` fails, and JSON Schema advertises an object (the
/// legacy `Value` shape renders as `true`/any). The v0.15 struct is unchanged
/// for wire compatibility; convert with `From`/`TryFrom` below.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CanonicalUploadMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata_json: crate::MetadataObject,
}

impl From<CanonicalUploadMetadata> for crate::LibraryFileUploadMetadata {
    fn from(canonical: CanonicalUploadMetadata) -> Self {
        Self {
            external_id: canonical.external_id,
            source_uri: canonical.source_uri,
            published_at: canonical.published_at,
            metadata_json: crate::metadata_object_to_value(&canonical.metadata_json),
        }
    }
}

impl TryFrom<crate::LibraryFileUploadMetadata> for CanonicalUploadMetadata {
    type Error = anyhow::Error;

    fn try_from(legacy: crate::LibraryFileUploadMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            external_id: legacy.external_id,
            source_uri: legacy.source_uri,
            published_at: legacy.published_at,
            metadata_json: crate::strict_metadata_object(&legacy.metadata_json)?,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct IngestOptions {
    #[serde(default)]
    pub metadata: CanonicalUploadMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<crate::ExtractionDirective>,
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

    pub fn from_legacy(
        metadata: Option<crate::LibraryFileUploadMetadata>,
        translation: Option<crate::TranslationDirective>,
        extraction: Option<crate::ExtractionDirective>,
        delete_source_after_processing: bool,
    ) -> Self {
        let metadata = metadata
            .and_then(|legacy| CanonicalUploadMetadata::try_from(legacy).ok())
            .unwrap_or_default();
        Self {
            metadata,
            translation,
            extraction,
            source_policy: SourcePolicy::from_delete_flag(delete_source_after_processing),
        }
    }
}
