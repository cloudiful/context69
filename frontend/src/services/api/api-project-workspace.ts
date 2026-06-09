import type {
  CreateFolderRequest,
  LibraryUploadResponse,
  MoveFileRequest,
  MoveFolderRequest,
  RequestOptions,
  SourceConfigInput,
} from "./api-types";

type Deps = {
  authFetch: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
  openapiClient: import("./api-core").OpenApiClient;
  resolveApiUrl: (path: string) => string;
  unwrapFetchResponse: <TData>(response: Response) => Promise<TData>;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createProjectWorkspaceApi({
  authFetch,
  openapiClient,
  resolveApiUrl,
  unwrapFetchResponse,
  unwrapResponse,
}: Deps) {
  return {
    listProjectSources(groupKey: string, projectKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects/{project_key}/sources", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        signal: options?.signal,
      }));
    },
    createProjectSource(groupKey: string, projectKey: string, payload: SourceConfigInput, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/sources", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    updateProjectSource(groupKey: string, projectKey: string, sourceKey: string, payload: SourceConfigInput, options?: RequestOptions) {
      return unwrapResponse(openapiClient.PUT("/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}", {
        params: { path: { group_key: groupKey, project_key: projectKey, source_key: sourceKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteProjectSource(groupKey: string, projectKey: string, sourceKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}", {
        params: { path: { group_key: groupKey, project_key: projectKey, source_key: sourceKey } },
        signal: options?.signal,
      }));
    },
    syncProjectSource(groupKey: string, projectKey: string, sourceKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/sources/{source_key}/sync", {
        params: { path: { group_key: groupKey, project_key: projectKey, source_key: sourceKey } },
        signal: options?.signal,
      }));
    },
    getProjectLibraryTree(groupKey: string, projectKey: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects/{project_key}/library/tree", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        signal: options?.signal,
      }));
    },
    createProjectLibraryFolder(groupKey: string, projectKey: string, payload: CreateFolderRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/library/folders", {
        params: { path: { group_key: groupKey, project_key: projectKey } },
        body: payload,
        signal: options?.signal,
      }));
    },
    moveProjectLibraryFolder(groupKey: string, projectKey: string, folderId: string, payload: MoveFolderRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}/move", {
        params: { path: { group_key: groupKey, project_key: projectKey, folder_id: folderId } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteProjectLibraryFolder(groupKey: string, projectKey: string, folderId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}/projects/{project_key}/library/folders/{folder_id}", {
        params: { path: { group_key: groupKey, project_key: projectKey, folder_id: folderId } },
        signal: options?.signal,
      }));
    },
    async uploadProjectLibraryFiles(groupKey: string, projectKey: string, folderId: string | null, files: File[], options?: RequestOptions) {
      const form = new FormData();
      if (folderId) {
        form.append("folder_id", folderId);
      }
      for (const file of files) {
        form.append("files", file);
      }

      const response = await authFetch(resolveApiUrl(`/v1/groups/${groupKey}/projects/${projectKey}/library/files/upload`), {
        body: form,
        method: "POST",
        signal: options?.signal,
      });

      return unwrapFetchResponse<LibraryUploadResponse>(response);
    },
    getProjectLibraryFile(groupKey: string, projectKey: string, fileId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}", {
        params: { path: { group_key: groupKey, project_key: projectKey, file_id: fileId } },
        signal: options?.signal,
      }));
    },
    moveProjectLibraryFile(groupKey: string, projectKey: string, fileId: string, payload: MoveFileRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}/move", {
        params: { path: { group_key: groupKey, project_key: projectKey, file_id: fileId } },
        body: payload,
        signal: options?.signal,
      }));
    },
    deleteProjectLibraryFile(groupKey: string, projectKey: string, fileId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/groups/{group_key}/projects/{project_key}/library/files/{file_id}", {
        params: { path: { group_key: groupKey, project_key: projectKey, file_id: fileId } },
        signal: options?.signal,
      }));
    },
    getProjectLibraryJob(groupKey: string, projectKey: string, jobId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/groups/{group_key}/projects/{project_key}/library/jobs/{job_id}", {
        params: { path: { group_key: groupKey, project_key: projectKey, job_id: jobId } },
        signal: options?.signal,
      }));
    },
  };
}
