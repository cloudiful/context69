import type {
  CreateFolderRequest,
  CreateSourceFolderRequest,
  LibraryUploadResponse,
  MoveFileRequest,
  MoveFolderRequest,
  RequestOptions,
  SourceConfigInput,
  UpsertLibraryTextRequest,
  LibraryResourceSortBy,
  SortDirection,
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
      const form = new FormData();
      if (folderId) {
        form.append("folder_id", folderId);
      }
      for (const file of files) {
        form.append("files", file);
      }

      const response = await authFetch(resolveApiUrl(`/v1/groups/by-path/${encodeGroupPath(groupPath)}/library/files/upload`), {
        body: form,
        method: "POST",
        signal: options?.signal,
      });

      return unwrapFetchResponse<LibraryUploadResponse>(response);
    },
    getGroupLibraryFile(groupPath: string, fileId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/library/files/{file_id}", {
        params: { path: { group_path: groupPath, file_id: fileId } },
        signal: options?.signal,
      }));
    },
    retryGroupLibraryFile(groupPath: string, fileId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/by-path/{group_path}/library/files/{file_id}/retry", {
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
    getGroupLibraryJob(groupPath: string, jobId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/by-path/{group_path}/library/jobs/{job_id}", {
        params: { path: { group_path: groupPath, job_id: jobId } },
        signal: options?.signal,
      }));
    },
  };
}
