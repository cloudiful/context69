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
    LibraryResourceItem, LibraryResourceKind, LibraryResourcePageQuery,
    LibraryResourcePageResponse, LibraryResourceSortBy, LibraryTextContentFormat,
    LibraryTreeResponse, LibraryUploadResponse, MoveFileRequest, MoveFolderRequest, SortDirection,
    UpsertLibraryTextRequest,
};
pub use mcp::McpDocumentArgs;
pub use namespace::{
    CreateGroupRequest, GroupKind, GroupMemberResponse, GroupResponse, MembershipRole,
    MoveGroupRequest, UpdateGroupRequest, UpsertMembershipRequest, UserDirectoryEntryResponse,
    Visibility,
};
pub use search::{
    DocumentChunkResponse, DocumentResponse, SearchHit, SearchMode, SearchRequest, SearchResponse,
};
pub use settings::{
    DoclingConnectionSettingsResponse, DoclingSettingsResponse, DoclingSettingsSource,
    DoclingVlmSettingsResponse, RuntimeChunkingSettings, RuntimeEmbeddingSettings,
    RuntimeFileLibrarySettings, RuntimeQdrantSettings, RuntimeSchedulerSettings,
    RuntimeSettingsResponse, SearchSettingsResponse, UpdateDoclingConnectionSettings,
    UpdateDoclingSettingsRequest, UpdateDoclingVlmSettings, UpdateRuntimeEmbeddingSettings,
    UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
};
pub use sources::{
    CreateSourceFolderRequest, ListSourcesResponse, SourceConfigInput, SourceConnectionResponse,
    SourceFolderResponse, SourceOriginStatusKind, SourceStatus, SyncOutcome,
    UpsertSourceConnectionRequest,
};
