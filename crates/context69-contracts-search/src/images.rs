use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::documents::DocumentKey;

/// The largest accepted visual search query, in characters.
pub const IMAGE_SEARCH_QUERY_MAX_CHARS: usize = 200;

/// The default image search result limit.
pub const IMAGE_SEARCH_LIMIT_DEFAULT: usize = 20;

/// The largest accepted image search result limit.
pub const IMAGE_SEARCH_LIMIT_MAX: usize = 50;

/// The largest accepted document image unlink batch.
pub const IMAGE_UNLINK_ITEMS_MAX: usize = 100;

/// The largest number of image occurrences one document list returns.
pub const IMAGE_DOCUMENT_LIST_MAX: usize = 512;

/// How one occurrence uses its image.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImageOccurrenceRole {
    Cover,
    #[default]
    Body,
}

impl ImageOccurrenceRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Body => "body",
        }
    }
}

impl std::str::FromStr for ImageOccurrenceRole {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "cover" => Ok(Self::Cover),
            "body" => Ok(Self::Body),
            other => Err(format!(
                "unknown image occurrence role {other}; expected cover or body"
            )),
        }
    }
}

/// One visual search request inside a group scope.
///
/// `limit` is the number of image points requested. Hydration may return fewer
/// points when a candidate is stale; there is no total or pagination, so a
/// caller starts a new search instead of following a cursor.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ImageSearchRequest {
    #[schema(min_length = 1, max_length = 200)]
    pub query: String,
    #[serde(default = "default_image_search_limit")]
    #[schema(minimum = 1, maximum = 50)]
    pub limit: usize,
    /// Optional supported image media type filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

pub const fn default_image_search_limit() -> usize {
    IMAGE_SEARCH_LIMIT_DEFAULT
}

impl ImageSearchRequest {
    /// Validate the query and result limit before any provider call.
    pub fn validate(&self) -> Result<(), String> {
        let query = self.query.trim();
        if query.is_empty() {
            return Err("image search query must not be empty".to_string());
        }
        if query.chars().count() > IMAGE_SEARCH_QUERY_MAX_CHARS {
            return Err(format!(
                "image search query must be at most {IMAGE_SEARCH_QUERY_MAX_CHARS} characters"
            ));
        }
        if self.limit == 0 || self.limit > IMAGE_SEARCH_LIMIT_MAX {
            return Err(format!(
                "image search limit must be between 1 and {IMAGE_SEARCH_LIMIT_MAX}"
            ));
        }
        Ok(())
    }
}

/// One current parent occurrence of a search-hit image.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentImageOccurrenceWithParent {
    pub document_id: i64,
    pub document_key: DocumentKey,
    pub title: String,
    pub document_source_uri: String,
    pub role: ImageOccurrenceRole,
    pub ordinal: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The page or source that displayed this image occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

/// One hydrated image search hit.
///
/// `parents` is empty for a standalone image. `has_more_parents` reports that
/// the bounded parent hydration stopped before every current link was read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ImageSearchHit {
    pub score: f32,
    pub file_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub sha256: String,
    pub parents: Vec<DocumentImageOccurrenceWithParent>,
    pub has_more_parents: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ImageSearchResponse {
    pub items: Vec<ImageSearchHit>,
}

/// One current image occurrence of a document.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentImageOccurrence {
    pub file_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub sha256: String,
    pub role: ImageOccurrenceRole,
    pub ordinal: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentImageListResponse {
    pub document_id: i64,
    pub items: Vec<DocumentImageOccurrence>,
    /// The bounded list stopped before every occurrence was read.
    pub has_more: bool,
}

/// One occurrence link to remove.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UnlinkDocumentImageItem {
    pub file_id: Uuid,
    pub document_id: i64,
    pub role: ImageOccurrenceRole,
    pub ordinal: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UnlinkDocumentImagesRequest {
    #[schema(max_items = 100)]
    pub items: Vec<UnlinkDocumentImageItem>,
}

impl UnlinkDocumentImagesRequest {
    /// Validate the bounded batch before any link is removed.
    pub fn validate(&self) -> Result<(), String> {
        if self.items.is_empty() {
            return Err("image unlink items must not be empty".to_string());
        }
        if self.items.len() > IMAGE_UNLINK_ITEMS_MAX {
            return Err(format!(
                "image unlink items must contain at most {IMAGE_UNLINK_ITEMS_MAX} entries"
            ));
        }
        if self.items.iter().any(|item| item.ordinal < 0) {
            return Err("the image occurrence ordinal must not be negative".to_string());
        }
        Ok(())
    }
}

/// The per-item result of one unlink request.
///
/// A missing occurrence is idempotent success and reports `not_linked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageUnlinkOutcome {
    Unlinked,
    NotLinked,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UnlinkDocumentImageResult {
    pub file_id: Uuid,
    pub document_id: i64,
    pub role: ImageOccurrenceRole,
    pub ordinal: i32,
    pub outcome: ImageUnlinkOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UnlinkDocumentImagesResponse {
    pub items: Vec<UnlinkDocumentImageResult>,
}

#[cfg(test)]
mod tests {
    use super::{
        IMAGE_SEARCH_LIMIT_DEFAULT, IMAGE_SEARCH_QUERY_MAX_CHARS, IMAGE_UNLINK_ITEMS_MAX,
        ImageOccurrenceRole, ImageSearchRequest, UnlinkDocumentImageItem,
        UnlinkDocumentImagesRequest,
    };
    use uuid::Uuid;

    #[test]
    fn occurrence_roles_use_one_snake_case_vocabulary() {
        for (value, role) in [
            ("cover", ImageOccurrenceRole::Cover),
            ("body", ImageOccurrenceRole::Body),
        ] {
            let json = serde_json::to_string(&role).expect("serialize role");
            assert_eq!(json, format!("\"{value}\""));
            let parsed: ImageOccurrenceRole =
                serde_json::from_str(&json).expect("deserialize role");
            assert_eq!(parsed, role);
        }
        assert_eq!(
            " cover ".parse::<ImageOccurrenceRole>(),
            Ok(ImageOccurrenceRole::Cover)
        );
        assert!(" thumbnail ".parse::<ImageOccurrenceRole>().is_err());
        assert_eq!(ImageOccurrenceRole::default(), ImageOccurrenceRole::Body);
    }

    #[test]
    fn visual_search_defaults_and_bounds_are_explicit() {
        let request: ImageSearchRequest =
            serde_json::from_str(r#"{"query":"a red bicycle"}"#).expect("deserialize request");
        assert_eq!(request.limit, IMAGE_SEARCH_LIMIT_DEFAULT);
        assert!(request.validate().is_ok());

        let empty = ImageSearchRequest {
            query: "   ".to_string(),
            limit: 1,
            media_type: None,
        };
        assert!(empty.validate().is_err());

        let too_long = ImageSearchRequest {
            query: "q".repeat(IMAGE_SEARCH_QUERY_MAX_CHARS + 1),
            limit: 1,
            media_type: None,
        };
        assert!(too_long.validate().is_err());

        for limit in [0, 51] {
            assert!(
                ImageSearchRequest {
                    query: "query".to_string(),
                    limit,
                    media_type: None,
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn unlink_batches_are_bounded_and_keep_per_item_identity() {
        let item = UnlinkDocumentImageItem {
            file_id: Uuid::nil(),
            document_id: 4,
            role: ImageOccurrenceRole::Cover,
            ordinal: 0,
        };
        assert!(
            UnlinkDocumentImagesRequest {
                items: vec![item.clone()]
            }
            .validate()
            .is_ok()
        );
        assert!(
            UnlinkDocumentImagesRequest { items: Vec::new() }
                .validate()
                .is_err()
        );
        assert!(
            UnlinkDocumentImagesRequest {
                items: vec![item; IMAGE_UNLINK_ITEMS_MAX + 1]
            }
            .validate()
            .is_err()
        );
    }
}
