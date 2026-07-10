import type { components } from "../../generated/openapi";

type ApiSchemas = components["schemas"];
type MembershipRole = "owner" | "maintainer" | "viewer";
type Visibility = "public" | "private";
type GroupKind = "personal" | "shared";
type SearchModeValue = "vector" | "hybrid";

type SchemaOr<TName extends string, TFallback> =
  TName extends keyof ApiSchemas ? ApiSchemas[TName] : TFallback;

export type ApiErrorResponse = components["schemas"]["ApiErrorResponse"];
export type AdminUserResponse = components["schemas"]["AdminUserResponse"];
export type AuthLoginRequest = components["schemas"]["AuthLoginRequest"];
export type AuthMeResponse = components["schemas"]["AuthMeResponse"];
export type AuthTokenResponse = components["schemas"]["AuthTokenResponse"];
export type AuthUserResponse = components["schemas"]["AuthUserResponse"];
export type CreateAdminUserRequest = components["schemas"]["CreateAdminUserRequest"];
export type CreatePersonalAccessTokenRequest = components["schemas"]["CreatePersonalAccessTokenRequest"];
export type CreatePersonalAccessTokenResponse = components["schemas"]["CreatePersonalAccessTokenResponse"];
export type CreateFolderRequest = components["schemas"]["CreateFolderRequest"];
export type CreateSourceFolderRequest = components["schemas"]["CreateSourceFolderRequest"];
export type CreateGroupRequest = SchemaOr<"CreateGroupRequest", {
  parent_group_path?: string | null;
  group_key: string;
  name: string;
  visibility: Visibility;
  kind?: GroupKind | null;
}>;
export type DoclingSettingsResponse = SchemaOr<"DoclingSettingsResponse", {
  configured: boolean;
  source: "config" | "database" | "unconfigured";
  connection: {
    base_url?: string | null;
    timeout_secs: number;
    poll_interval_secs: number;
  };
  vlm: {
    provider_account_key?: string | null;
    openai_base_url?: string | null;
    has_api_key: boolean;
    vlm_pipeline_model?: string | null;
    picture_description_model?: string | null;
    code_formula_model?: string | null;
  };
}>;
export type DocumentChunkResponse = SchemaOr<"DocumentChunkResponse", {
  chunk_id: string;
  chunk_index: number;
  text: string;
}>;
export type DocumentResponse = SchemaOr<"DocumentResponse", {
  document_id: number;
  group_key: string;
  group_path: string;
  visibility: Visibility;
  source_key: string;
  external_id: string;
  title: string;
  summary?: string | null;
  source_uri: string;
  published_at?: string | null;
  updated_at: string;
  record_hash: string;
  metadata_json: Record<string, unknown>;
  library_file_id?: string | null;
  library_section_label?: string | null;
  library_path?: string | null;
  is_library_file: boolean;
  chunks: DocumentChunkResponse[];
}>;
export type GroupMemberResponse = SchemaOr<"GroupMemberResponse", {
  user_id: number;
  login_name: string;
  display_name: string;
  role: MembershipRole;
}>;
export type GroupResponse = SchemaOr<"GroupResponse", {
  group_id: number;
  group_key: string;
  group_path?: string | null;
  parent_group_path?: string | null;
  name: string;
  visibility: Visibility;
  kind: GroupKind;
  current_role?: MembershipRole | null;
  created_at: string;
  updated_at: string;
}>;
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
export type MoveGroupRequest = SchemaOr<"MoveGroupRequest", {
  target_parent_group_path?: string | null;
}>;
export type PersonalAccessTokenResponse = components["schemas"]["PersonalAccessTokenResponse"];
export type PersonalAccessTokenScope = components["schemas"]["PersonalAccessTokenScope"];
export type ProviderAccountResponse = SchemaOr<"ProviderAccountResponse", {
  account_key: string;
  provider_kind: string;
  display_name: string;
  base_url: string;
  has_api_key: boolean;
  disabled_at?: string | null;
}>;
export type ResetAdminUserPasswordRequest = components["schemas"]["ResetAdminUserPasswordRequest"];
export type RuntimeSettingsResponse = SchemaOr<"RuntimeSettingsResponse", {
  qdrant: {
    url: string;
    collection_name: string;
    recreate_on_dimension_mismatch: boolean;
  };
  embedding: {
    provider_account_key: string;
    model: string;
    dimensions: number;
    timeout_secs: number;
  };
  scheduler: {
    interval_secs: number;
    run_on_start: boolean;
    max_concurrency: number;
    job_id: string;
    valkey_url?: string | null;
  };
  chunking: {
    max_chars: number;
    overlap_chars: number;
  };
  file_library: {
    storage_root: string;
    max_upload_size_mb: number;
    max_upload_request_size_mb: number;
    ingest_concurrency: number;
    pdf_pages_per_task: number;
  };
}>;
export type SearchHit = SchemaOr<"SearchHit", {
  chunk_id: string;
  document_id: number;
  group_key: string;
  group_path: string;
  visibility: Visibility;
  source_key: string;
  external_id: string;
  title: string;
  summary?: string | null;
  source_uri: string;
  published_at?: string | null;
  chunk_index: number;
  chunk_text: string;
  score: number;
  vector_score?: number | null;
  keyword_score?: number | null;
  rerank_score?: number | null;
  match_reason?: string | null;
  metadata_json: Record<string, unknown>;
  library_file_id?: string | null;
  library_section_label?: string | null;
  library_path?: string | null;
  is_library_file?: boolean;
}>;
export type SearchRequest = SchemaOr<"SearchRequest", {
  query: string;
  limit: number;
  source_key?: string | null;
  group_path?: string | null;
  published_after?: string | null;
  published_before?: string | null;
}>;
export type SearchResponse = SchemaOr<"SearchResponse", {
  query: string;
  hits: SearchHit[];
}>;
export type SearchSettingsResponse = SchemaOr<"SearchSettingsResponse", {
  mode: SearchModeValue;
  rerank_enabled: boolean;
  rerank_base_url: string;
  rerank_model: string;
  candidate_limit: number;
  timeout_secs: number;
  has_api_key: boolean;
}>;
export type SourceConfigInput = components["schemas"]["SourceConfigInput"];
export type SourceConnectionResponse = components["schemas"]["SourceConnectionResponse"];
export type SourceFolderResponse = components["schemas"]["SourceFolderResponse"];
export type SourceStatus = components["schemas"]["SourceStatus"];
export type SyncOutcome = components["schemas"]["SyncOutcome"];
export type UpdateAdminUserRequest = components["schemas"]["UpdateAdminUserRequest"];
export type UpdateDoclingSettingsRequest = SchemaOr<"UpdateDoclingSettingsRequest", {
  connection: {
    base_url: string;
    timeout_secs: number;
    poll_interval_secs: number;
  };
  vlm: {
    provider_account_key?: string | null;
    openai_base_url?: string | null;
    api_key?: string | null;
    clear_api_key?: boolean;
    vlm_pipeline_model?: string | null;
    picture_description_model?: string | null;
    code_formula_model?: string | null;
  };
}>;
export type UpdateGroupRequest = SchemaOr<"UpdateGroupRequest", {
  name?: string | null;
  visibility?: Visibility | null;
}>;
export type UpdateSearchSettingsRequest = SchemaOr<"UpdateSearchSettingsRequest", {
  mode: SearchModeValue;
  rerank_enabled: boolean;
  rerank_base_url: string;
  rerank_model: string;
  candidate_limit: number;
  timeout_secs: number;
  api_key?: string | null;
  clear_api_key: boolean;
}>;
export type UpdateRuntimeSettingsRequest = SchemaOr<"UpdateRuntimeSettingsRequest", RuntimeSettingsResponse>;
export type UpsertLibraryTextRequest = components["schemas"]["UpsertLibraryTextRequest"];
export type UpsertMembershipRequest = SchemaOr<"UpsertMembershipRequest", {
  login_name: string;
  role: MembershipRole;
}>;
export type UpsertProviderAccountRequest = SchemaOr<"UpsertProviderAccountRequest", {
  account_key: string;
  provider_kind: string;
  display_name: string;
  base_url: string;
  api_key?: string | null;
  clear_api_key: boolean;
  disabled: boolean;
}>;
export type UpsertSourceConnectionRequest = components["schemas"]["UpsertSourceConnectionRequest"];
export type UserDirectoryEntryResponse = SchemaOr<"UserDirectoryEntryResponse", {
  user_id: number;
  login_name: string;
  display_name: string;
}>;

export interface RequestOptions {
  signal?: AbortSignal;
}

export type ApiResult<TData> = {
  data?: TData;
  error?: unknown;
  response: Response;
};
