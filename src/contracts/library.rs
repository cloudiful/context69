use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::Visibility;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryIngestStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

impl LibraryIngestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for LibraryIngestStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            other => Err(anyhow::anyhow!(
                "unsupported library ingest status: {other}"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateFolderRequest {
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveFolderRequest {
    #[serde(default)]
    pub target_folder_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveFileRequest {
    #[serde(default)]
    pub target_folder_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateTextRequest {
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileSummary {
    pub file_id: Uuid,
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub ingest_status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFolderNode {
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
    pub path: String,
    pub processing_count: usize,
    #[schema(no_recursion)]
    pub children: Vec<LibraryFolderNode>,
    pub files: Vec<LibraryFileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFolderResponse {
    pub folder_id: Uuid,
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryTreeResponse {
    pub root: LibraryFolderNode,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryIngestJobResponse {
    pub job_id: Uuid,
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    pub file_id: Uuid,
    pub status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docling_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPreviewContentFormat {
    PlainText,
    Markdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryDocumentSectionPreview {
    pub document_id: i64,
    pub section_key: String,
    pub section_label: String,
    pub sort_order: i32,
    pub title: String,
    pub preview_text: String,
    #[serde(default = "default_preview_content_format")]
    pub content_format: LibraryPreviewContentFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileDetailResponse {
    pub file_id: Uuid,
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub folder_path: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub ingest_status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<DateTime<Utc>>,
    pub sections: Vec<LibraryDocumentSectionPreview>,
    pub jobs: Vec<LibraryIngestJobResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryUploadResponse {
    pub files: Vec<LibraryFileSummary>,
    pub jobs: Vec<LibraryIngestJobResponse>,
}

fn default_preview_content_format() -> LibraryPreviewContentFormat {
    LibraryPreviewContentFormat::PlainText
}
