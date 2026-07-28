import type {
  CreateFolderRequest,
  LibraryIngestFailureStage,
  LibraryIngestStatus,
  LibraryFileJobPageResponse,
  LibraryProcessingJobBulkActionResponse,
  LibraryProcessingJobPageResponse,
  LibraryResourceSortBy,
  LibraryUploadResponse,
  MoveFileRequest,
  MoveFolderRequest,
  RequestOptions,
  SortDirection,
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
    getLibraryResources(params: {
      folderId: string | null;
      page: number;
      pageSize: number;
      query: string;
      status: LibraryIngestStatus | null;
      sortBy: LibraryResourceSortBy;
      sortDirection: SortDirection;
    }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/library/resources", {
        params: {
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
    getLibraryProcessingJobs(params: {
      page: number;
      pageSize: number;
      query: string;
      status: LibraryIngestStatus | null;
      failureStage: LibraryIngestFailureStage | null;
    }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/library/processing-jobs", {
        params: {
          query: {
            page: params.page,
            page_size: params.pageSize,
            query: params.query || undefined,
            status: params.status ?? undefined,
            failure_stage: params.failureStage ?? undefined,
          },
        },
        signal: options?.signal,
      })) as Promise<LibraryProcessingJobPageResponse>;
    },
    retryFailedLibraryProcessingJobs(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/library/processing-jobs/retry-failed", {
          body: {},
          signal: options?.signal,
        }),
      ) as Promise<LibraryProcessingJobBulkActionResponse>;
    },
    cleanupStuckLibraryProcessingJobs(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/library/processing-jobs/cleanup-stuck", {
          signal: options?.signal,
        }),
      ) as Promise<LibraryProcessingJobBulkActionResponse>;
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
    getLibraryFileJobs(fileId: string, params: { page: number; pageSize: number }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/library/files/{file_id}/jobs", {
        params: {
          path: { file_id: fileId },
          query: { page: params.page, page_size: params.pageSize },
        },
        signal: options?.signal,
      })) as Promise<LibraryFileJobPageResponse>;
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
