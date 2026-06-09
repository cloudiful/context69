mod auth;
mod common;
mod library;
mod mcp;
mod namespace;
mod search;
mod settings;
mod sources;

pub use auth::{
    AdminUserResponse, AuthLoginRequest, AuthMeResponse, AuthTokenResponse, AuthUserResponse,
    CreateAdminUserRequest, ResetAdminUserPasswordRequest, UpdateAdminUserRequest,
};
pub use common::{ApiErrorResponse, HealthResponse, HealthStatus};
pub use library::{
    CreateFolderRequest, CreateTextRequest, LibraryDocumentSectionPreview,
    LibraryFileDetailResponse, LibraryFileSummary, LibraryFolderNode, LibraryFolderResponse,
    LibraryIngestJobResponse, LibraryIngestStatus, LibraryPreviewContentFormat,
    LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest, MoveFolderRequest,
};
pub use mcp::McpDocumentArgs;
pub use namespace::{
    CreateGroupRequest, CreateProjectRequest, GroupKind, GroupMemberResponse, GroupResponse,
    MembershipRole, MoveProjectRequest, ProjectMemberResponse, ProjectResponse,
    UpdateGroupRequest, UpdateProjectRequest, UpsertMembershipRequest,
    UserDirectoryEntryResponse, Visibility,
};
pub use search::{
    DocumentChunkResponse, DocumentResponse, SearchHit, SearchMode, SearchRequest, SearchResponse,
};
pub use settings::{
    DoclingConnectionSettingsResponse, DoclingConversionSettings, DoclingEnrichmentSettings,
    DoclingOcrSettings, DoclingSettingsResponse, DoclingSettingsSource,
    DoclingVlmSettingsResponse, ProviderAccountResponse, RuntimeChunkingSettings,
    RuntimeEmbeddingSettings, RuntimeFileLibrarySettings, RuntimeQdrantSettings,
    RuntimeSchedulerSettings, RuntimeSettingsResponse, SearchSettingsResponse,
    UpdateDoclingConnectionSettings, UpdateDoclingSettingsRequest, UpdateDoclingVlmSettings,
    UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest, UpsertProviderAccountRequest,
};
pub use sources::{
    ListSourcesResponse, SourceConfigInput, SourceConnectionResponse, SourceOriginStatusKind,
    SourceStatus, SyncOutcome, UpsertSourceConnectionRequest,
};
