import type { components } from "../../generated/openapi";

export type ApiErrorResponse = components["schemas"]["ApiErrorResponse"];
export type AdminUserResponse = components["schemas"]["AdminUserResponse"];
export type AuthLoginRequest = components["schemas"]["AuthLoginRequest"];
export type AuthMeResponse = components["schemas"]["AuthMeResponse"];
export type AuthTokenResponse = components["schemas"]["AuthTokenResponse"];
export type AuthUserResponse = components["schemas"]["AuthUserResponse"];
export type CreateAdminUserRequest = components["schemas"]["CreateAdminUserRequest"];
export type CreateFolderRequest = components["schemas"]["CreateFolderRequest"];
export type CreateGroupRequest = components["schemas"]["CreateGroupRequest"];
export type CreateProjectRequest = components["schemas"]["CreateProjectRequest"];
export type DoclingSettingsResponse = components["schemas"]["DoclingSettingsResponse"];
export type DocumentChunkResponse = components["schemas"]["DocumentChunkResponse"];
export type DocumentResponse = components["schemas"]["DocumentResponse"];
export type GroupMemberResponse = components["schemas"]["GroupMemberResponse"];
export type GroupResponse = components["schemas"]["GroupResponse"];
export type HealthResponse = components["schemas"]["HealthResponse"];
export type LibraryDocumentSectionPreview = components["schemas"]["LibraryDocumentSectionPreview"];
export type LibraryFileDetailResponse = components["schemas"]["LibraryFileDetailResponse"];
export type LibraryFileSummary = components["schemas"]["LibraryFileSummary"];
export type LibraryFolderNode = components["schemas"]["LibraryFolderNode"];
export type LibraryFolderResponse = components["schemas"]["LibraryFolderResponse"];
export type LibraryIngestJobResponse = components["schemas"]["LibraryIngestJobResponse"];
export type LibraryIngestStatus = components["schemas"]["LibraryIngestStatus"];
export type LibraryPreviewContentFormat = components["schemas"]["LibraryPreviewContentFormat"];
export type LibraryTreeResponse = components["schemas"]["LibraryTreeResponse"];
export type LibraryUploadResponse = components["schemas"]["LibraryUploadResponse"];
export type MoveFileRequest = components["schemas"]["MoveFileRequest"];
export type MoveFolderRequest = components["schemas"]["MoveFolderRequest"];
export type MoveProjectRequest = components["schemas"]["MoveProjectRequest"];
export type ProjectMemberResponse = components["schemas"]["ProjectMemberResponse"];
export type ProjectResponse = components["schemas"]["ProjectResponse"];
export type ProviderAccountResponse = components["schemas"]["ProviderAccountResponse"];
export type ResetAdminUserPasswordRequest = components["schemas"]["ResetAdminUserPasswordRequest"];
export type RuntimeSettingsResponse = components["schemas"]["RuntimeSettingsResponse"];
export type SearchHit = components["schemas"]["SearchHit"];
export type SearchRequest = components["schemas"]["SearchRequest"];
export type SearchResponse = components["schemas"]["SearchResponse"];
export type SearchSettingsResponse = components["schemas"]["SearchSettingsResponse"];
export type SourceConfigInput = components["schemas"]["SourceConfigInput"];
export type SourceConnectionResponse = components["schemas"]["SourceConnectionResponse"];
export type SourceStatus = components["schemas"]["SourceStatus"];
export type SyncOutcome = components["schemas"]["SyncOutcome"];
export type UpdateAdminUserRequest = components["schemas"]["UpdateAdminUserRequest"];
export type UpdateDoclingSettingsRequest = components["schemas"]["UpdateDoclingSettingsRequest"];
export type UpdateGroupRequest = components["schemas"]["UpdateGroupRequest"];
export type UpdateProjectRequest = components["schemas"]["UpdateProjectRequest"];
export type UpdateSearchSettingsRequest = components["schemas"]["UpdateSearchSettingsRequest"];
export type UpdateRuntimeSettingsRequest = components["schemas"]["UpdateRuntimeSettingsRequest"];
export type UpsertLibraryTextRequest = components["schemas"]["UpsertLibraryTextRequest"];
export type UpsertMembershipRequest = components["schemas"]["UpsertMembershipRequest"];
export type UpsertProviderAccountRequest = components["schemas"]["UpsertProviderAccountRequest"];
export type UpsertSourceConnectionRequest = components["schemas"]["UpsertSourceConnectionRequest"];
export type UserDirectoryEntryResponse = components["schemas"]["UserDirectoryEntryResponse"];

export interface RequestOptions {
  signal?: AbortSignal;
}

export type ApiResult<TData> = {
  data?: TData;
  error?: unknown;
  response: Response;
};
