import type {
  CreateFolderRequest,
  LibraryUploadResponse,
  MoveFileRequest,
  MoveFolderRequest,
  RequestOptions,
} from "./api-types";

type Deps = {
  authFetch: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
  openapiClient: import("./api-core").OpenApiClient;
  resolveApiUrl: (path: string) => string;
  unwrapFetchResponse: <TData>(response: Response) => Promise<TData>;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createLibraryApi({
  authFetch,
  openapiClient,
  resolveApiUrl,
  unwrapFetchResponse,
  unwrapResponse,
}: Deps) {
  return {
    getLibraryTree(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/library/tree", {
          signal: options?.signal,
        }),
      );
    },
    createLibraryFolder(payload: CreateFolderRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/library/folders", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    moveLibraryFolder(folderId: string, payload: MoveFolderRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/library/folders/{folder_id}/move", {
          params: {
            path: {
              folder_id: folderId,
            },
          },
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    deleteLibraryFolder(folderId: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/library/folders/{folder_id}", {
          params: {
            path: {
              folder_id: folderId,
            },
          },
          signal: options?.signal,
        }),
      );
    },
    async uploadLibraryFiles(folderId: string | null, files: File[], options?: RequestOptions) {
      const form = new FormData();
      if (folderId) {
        form.append("folder_id", folderId);
      }
      for (const file of files) {
        form.append("files", file);
      }

      const response = await authFetch(resolveApiUrl("/v1/library/files/upload"), {
        body: form,
        method: "POST",
        signal: options?.signal,
      });

      return unwrapFetchResponse<LibraryUploadResponse>(response);
    },
    getLibraryFile(fileId: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/library/files/{file_id}", {
          params: {
            path: {
              file_id: fileId,
            },
          },
          signal: options?.signal,
        }),
      );
    },
    moveLibraryFile(fileId: string, payload: MoveFileRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/library/files/{file_id}/move", {
          params: {
            path: {
              file_id: fileId,
            },
          },
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    deleteLibraryFile(fileId: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/library/files/{file_id}", {
          params: {
            path: {
              file_id: fileId,
            },
          },
          signal: options?.signal,
        }),
      );
    },
    getLibraryJob(jobId: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/library/jobs/{job_id}", {
          params: {
            path: {
              job_id: jobId,
            },
          },
          signal: options?.signal,
        }),
      );
    },
  };
}
