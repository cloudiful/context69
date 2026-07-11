import type { components } from "../../generated/openapi";

type Schemas = components["schemas"];

export type ApiErrorResponse = Schemas["ApiErrorResponse"];
export type AdminUserResponse = Schemas["AdminUserResponse"];
export type AuthLoginRequest = Schemas["AuthLoginRequest"];
export type AuthMeResponse = Schemas["AuthMeResponse"];
export type AuthTokenResponse = Schemas["AuthTokenResponse"];
export type AuthUserResponse = Schemas["AuthUserResponse"];
export type CreateAdminUserRequest = Schemas["CreateAdminUserRequest"];
export type CreateFolderRequest = Schemas["CreateFolderRequest"];
export type CreateGroupRequest = Schemas["CreateGroupRequest"];
export type CreatePersonalAccessTokenRequest = Schemas["CreatePersonalAccessTokenRequest"];
export type CreatePersonalAccessTokenResponse = Schemas["CreatePersonalAccessTokenResponse"];
export type CreateSourceFolderRequest = Schemas["CreateSourceFolderRequest"];
export type DoclingSettingsResponse = Schemas["DoclingSettingsResponse"];
export type DocumentChunkResponse = Schemas["DocumentChunkResponse"];
export type DocumentResponse = Schemas["DocumentResponse"];
export type GroupMemberResponse = Schemas["GroupMemberResponse"];
export type GroupKind = Schemas["GroupKind"];
export type GroupResponse = Schemas["GroupResponse"];
export type HealthResponse = Schemas["HealthResponse"];
export type LibraryDocumentSectionPreview = Schemas["LibraryDocumentSectionPreview"];
export type LibraryFileDetailResponse = Schemas["LibraryFileDetailResponse"];
export type LibraryFileSummary = Schemas["LibraryFileSummary"];
export type LibraryFolderNode = Schemas["LibraryFolderNode"];
export type LibraryFolderResponse = Schemas["LibraryFolderResponse"];
export type LibraryIngestJobResponse = Schemas["LibraryIngestJobResponse"];
export type LibraryIngestStatus = Schemas["LibraryIngestStatus"];
export type LibraryPreviewContentFormat = Schemas["LibraryPreviewContentFormat"];
export type LibraryResourceItem = Schemas["LibraryResourceItem"];
export type LibraryResourcePageResponse = Schemas["LibraryResourcePageResponse"];
export type LibraryResourceSortBy = Schemas["LibraryResourceSortBy"];
export type SortDirection = Schemas["SortDirection"];
export type LibraryTreeResponse = Schemas["LibraryTreeResponse"];
export type LibraryUploadResponse = Schemas["LibraryUploadResponse"];
export type MoveFileRequest = Schemas["MoveFileRequest"];
export type MoveFolderRequest = Schemas["MoveFolderRequest"];
export type MoveGroupRequest = Schemas["MoveGroupRequest"];
export type MembershipRole = Schemas["MembershipRole"];
export type PersonalAccessTokenResponse = Schemas["PersonalAccessTokenResponse"];
export type PersonalAccessTokenScope = Schemas["PersonalAccessTokenScope"];
export type ResetAdminUserPasswordRequest = Schemas["ResetAdminUserPasswordRequest"];
export type RuntimeSettingsResponse = Schemas["RuntimeSettingsResponse"];
export type SearchHit = Schemas["SearchHit"];
export type SearchMode = Schemas["SearchMode"];
export type SearchRequest = Schemas["SearchRequest"];
export type SearchResponse = Schemas["SearchResponse"];
export type SearchSettingsResponse = Schemas["SearchSettingsResponse"];
export type SourceConfigInput = Schemas["SourceConfigInput"];
export type SourceConnectionResponse = Schemas["SourceConnectionResponse"];
export type SourceFolderResponse = Schemas["SourceFolderResponse"];
export type SourceStatus = Schemas["SourceStatus"];
export type SyncOutcome = Schemas["SyncOutcome"];
export type UpdateAdminUserRequest = Schemas["UpdateAdminUserRequest"];
export type UpdateDoclingSettingsRequest = Schemas["UpdateDoclingSettingsRequest"];
export type UpdateGroupRequest = Schemas["UpdateGroupRequest"];
export type UpdateRuntimeSettingsRequest = Schemas["UpdateRuntimeSettingsRequest"];
export type UpdateSearchSettingsRequest = Schemas["UpdateSearchSettingsRequest"];
export type UpsertLibraryTextRequest = Schemas["UpsertLibraryTextRequest"];
export type UpsertMembershipRequest = Schemas["UpsertMembershipRequest"];
export type UpsertSourceConnectionRequest = Schemas["UpsertSourceConnectionRequest"];
export type UserDirectoryEntryResponse = Schemas["UserDirectoryEntryResponse"];
export type Visibility = Schemas["Visibility"];

export interface RequestOptions {
  signal?: AbortSignal;
}

export type ApiResult<TData> = {
  data?: TData;
  error?: unknown;
  response: Response;
};
