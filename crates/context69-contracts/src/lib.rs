pub mod auth;
pub mod common;
pub mod library;
pub mod mcp;
pub mod namespace;
pub mod search;
pub mod settings;
pub mod sources;

pub use auth::{
    AdminUserResponse, AuthLoginRequest, AuthMeResponse, AuthTokenResponse, AuthUserResponse,
    CreateAdminUserRequest, CreatePersonalAccessTokenRequest, CreatePersonalAccessTokenResponse,
    PersonalAccessTokenResponse, PersonalAccessTokenScope, ResetAdminUserPasswordRequest,
    UpdateAdminUserRequest,
};
pub use common::{ApiErrorResponse, HealthResponse, HealthStatus};
pub use library::{
    CreateFolderRequest, CreateTextRequest, LibraryDocumentSectionPreview,
    LibraryFileDetailResponse, LibraryFileSummary, LibraryFolderNode, LibraryFolderResponse,
    LibraryIngestJobResponse, LibraryIngestStatus, LibraryPreviewContentFormat,
    LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest, MoveFolderRequest,
    UpsertLibraryTextRequest,
};
pub use mcp::McpDocumentArgs;
pub use namespace::{
    CreateGroupRequest, CreateProjectRequest, GroupKind, GroupMemberResponse, GroupResponse,
    MembershipRole, MoveProjectRequest, ProjectMemberResponse, ProjectResponse, UpdateGroupRequest,
    UpdateProjectRequest, UpsertMembershipRequest, UserDirectoryEntryResponse, Visibility,
};
pub use search::{
    DocumentChunkResponse, DocumentResponse, SearchHit, SearchMode, SearchRequest, SearchResponse,
};
pub use settings::{
    DoclingConnectionSettingsResponse, DoclingSettingsResponse, DoclingSettingsSource,
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
