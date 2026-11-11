use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ingest::SourcePolicy;

/// The library defaults one group persists for future uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GroupLibrarySettingsResponse {
    pub default_source_policy: SourcePolicy,
}

/// Replaces the library defaults one group persists for future uploads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateGroupLibrarySettingsRequest {
    pub default_source_policy: SourcePolicy,
}
