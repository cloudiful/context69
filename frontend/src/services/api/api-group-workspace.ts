import type {
  CreateFolderRequest,
  CreateSourceFolderRequest,
  MoveFileRequest,
  MoveFolderRequest,
  RequestOptions,
  SourceConfigInput,
  UpsertLibraryTextRequest,
  LibraryResourceSortBy,
  LibraryIngestStatus,
  SortDirection,
  CreateMetadataIndexRequest,
  UpdateMetadataIndexRequest,
  UpdateGroupTranslationSettingsRequest,
  TaskRef,
} from "./api-types";

type Deps = {
  authFetch: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
  openapiClient: import("./api-core").OpenApiClient;
  resolveApiUrl: (path: string) => string;
  unwrapFetchResponse: <TData>(response: Response) => Promise<TData>;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

function encodeGroupPath(groupPath: string) {
  return encodeURIComponent(groupPath);
}

export function createGroupWorkspaceApi({
  authFetch,
  openapiClient,
  resolveApiUrl,
  unwrapFetchResponse,
  unwrapResponse,
}: Deps) {
  return {
    getGroupTranslationSettings(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/translation-settings", {
        params: { path: { group_path: groupPath } },
        signal: options?.signal,
      }));
    },
    updateGroupTranslationSettings(groupPath: string, payload: UpdateGroupTranslationSettingsRequest) {
      return unwrapResponse(openapiClient.PUT("/v1/groups/by-path/{group_path}/translation-settings", {
        params: { path: { group_path: groupPath } },
        body: payload,
      }));
    },
    listMetadataIndexes(groupPath: string, sourceKey: string, params: { page: number; pageSize: number }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/metadata-indexes", {
        params: {
          path: { group_path: groupPath },
          query: { source_key: sourceKey, page: params.page, page_size: params.pageSize },
        }, signal: options?.signal,
      }));
    },
    createMetadataIndex(groupPath: string, sourceKey: string, payload: CreateMetadataIndexRequest) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/metadata-indexes", {
        params: { path: { group_path: groupPath }, query: { source_key: sourceKey } }, body: payload,
      }));
    },
    updateMetadataIndex(groupPath: string, indexId: string, payload: UpdateMetadataIndexRequest) {
      return unwrapResponse(openapiClient.PUT("/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}", {
        params: { path: { group_path: groupPath, index_id: indexId } }, body: payload,
      }));
    },
    retryMetadataIndex(groupPath: string, indexId: string) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}/retry", {
        params: { path: { group_path: groupPath, index_id: indexId } },
      }));
    },
    deleteMetadataIndex(groupPath: string, indexId: string) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}", {
        params: { path: { group_path: groupPath, index_id: indexId } },
      }));
    },
    createGroupSourceFolder(groupPath: string, payload: CreateSourceFolderRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/source-folders", {
        params: { path: { group_path: groupPath } },
        body: payload,
        signal: options?.signal,
      }));
    },
    updateGroupSourceFolderConfig(groupPath: string, folderId: string, payload: SourceConfigInput, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PUT("/v1/groups/by-path/{group_path}/source-folders/{folder_id}/config", {
        params: { path: { group_path: groupPath, folder_id: folderId } },
        body: payload,
        signal: options?.signal,
      }));
    },
    syncGroupSourceFolder(groupPath: string, folderId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/source-folders/{folder_id}/sync", {
        params: { path: { group_path: groupPath, folder_id: folderId } },
        signal: options?.signal,
      }));
    },
    getGroupLibraryTree(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/library/tree", {
        params: { path: { group_path: groupPath } },
        signal: options?.signal,
      }));
    },
    getGroupLibraryResources(groupPath: string, params: {
      folderId: string | null;
      page: number;
      pageSize: number;
      query: string;
      status: LibraryIngestStatus | null;
      sortBy: LibraryResourceSortBy;
      sortDirection: SortDirection;
    }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/library/resources", {
        params: {
          path: { group_path: groupPath },
          query: {
            folder_id: params.folderId ?? undefined,
            page: params.page,
            page_size: params.pageSize,
            query: params.query || undefined,
            status: params.status ?? undefined,
            sort_by: params.sortBy,
            sort_direction: params.sortDirection,
          },
        },
        signal: options?.signal,
      }));
    },
    upsertGroupLibraryText(groupPath: string, payload: UpsertLibraryTextRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PUT("/v1/groups/by-path/{group_path}/library/texts", {
        params: { path: { group_path: groupPath } },
        body: payload,
        signal: options?.signal,
      }));
    },
    createGroupLibraryFolder(groupPath: string, payload: CreateFolderRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/library/folders", {
        params: { path: { group_path: groupPath } },
        body: payload,
        signal: options?.signal,
      }));
    },
    moveGroupLibraryFolder(groupPath: string, folderId: string, payload: MoveFolderRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/library/folders/{folder_id}/move", {
        params: { path: { group_path: groupPath, folder_id: folderId } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteGroupLibraryFolder(groupPath: string, folderId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/by-path/{group_path}/library/folders/{folder_id}", {
        params: { path: { group_path: groupPath, folder_id: folderId } },
        signal: options?.signal,
      }));
    },
    async uploadGroupLibraryFiles(groupPath: string, folderId: string | null, files: File[], options?: RequestOptions) {
      const prepared = await Promise.all(files.map(async (file) => {
        const digest = await crypto.subtle.digest("SHA-256", await file.arrayBuffer());
        const sha256 = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
        const result = await unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/library/files/prepare-upload", {
          params: { path: { group_path: groupPath } },
          body: {
            folder_id: folderId,
            filename: file.name,
            media_type: file.type || "application/octet-stream",
            size_bytes: file.size,
            sha256,
          },
          signal: options?.signal,
        }));
        return { file, result, sha256 };
      }));
      const reused = prepared.filter(({ result }) => !result.upload_required);
      const pending = prepared.filter(({ result }) => result.upload_required).map(({ file }) => file);
      if (pending.length === 0) {
        return {
          files: reused.flatMap(({ result }) => result.file ? [result.file] : []),
          tasks: reused.flatMap(({ result }) => result.task ? [result.task] : []),
        };
      }
      const form = new FormData();
      if (folderId) {
        form.append("folder_id", folderId);
      }
      for (const { file, sha256 } of prepared.filter(({ result }) => result.upload_required)) {
        form.append("sha256", sha256);
        form.append("files", file);
      }

      const response = await authFetch(resolveApiUrl(`/v1/groups/by-path/${encodeGroupPath(groupPath)}/library/files/upload`), {
        body: form,
        method: "POST",
        signal: options?.signal,
      });

      const uploaded = await unwrapFetchResponse<TaskRef>(response);
      return {
        files: reused.flatMap(({ result }) => result.file ? [result.file] : []),
        tasks: [...reused.flatMap(({ result }) => result.task ? [result.task] : []), uploaded],
      };
    },
    getGroupLibraryFile(groupPath: string, fileId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/library/files/{file_id}", {
        params: { path: { group_path: groupPath, file_id: fileId } },
        signal: options?.signal,
      }));
    },
    moveGroupLibraryFile(groupPath: string, fileId: string, payload: MoveFileRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/library/files/{file_id}/move", {
        params: { path: { group_path: groupPath, file_id: fileId } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteGroupLibraryFile(groupPath: string, fileId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/by-path/{group_path}/library/files/{file_id}", {
        params: { path: { group_path: groupPath, file_id: fileId } },
        signal: options?.signal,
      }));
    },
  };
}
