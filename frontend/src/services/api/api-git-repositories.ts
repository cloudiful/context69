import type {
  GitCodeSearchResponse,
  GitIndexRequest,
  GitRepositoryConnectionRequest,
  GitRepositoryFileContentResponse,
  GitRepositoryFileDetailResponse,
  GitRepositoryFileListResponse,
  GitRepositoryRegistrationRequest,
  GitRepositorySettingsUpdateRequest,
  GitRepositorySource,
  RequestOptions,
  TaskRef,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export type GitRepositoryFilePageOptions = {
  limit?: number;
  cursor?: string;
};

export type GitRepositoryContentOptions = {
  path: string;
  startLine: number;
  endLine: number;
  cursor?: string;
};

export type GitRepositoryCodeSearchOptions = {
  query: string;
  pathPrefix?: string;
  language?: string;
  limit?: number;
};

export function createGitRepositoriesApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listGitRepositories(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/git-repositories", {
          params: { path: { group_path: groupPath } },
          signal: options?.signal,
        }),
      ) as Promise<GitRepositorySource[]>;
    },
    getGitRepository(groupPath: string, repositoryKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          signal: options?.signal,
        }),
      ) as Promise<GitRepositorySource>;
    },
    deleteGitRepository(groupPath: string, repositoryKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          signal: options?.signal,
        }),
      ) as Promise<void>;
    },
    registerGitRepository(
      groupPath: string,
      body: GitRepositoryRegistrationRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.POST("/v1/groups/by-path/{group_path}/git-repositories", {
          params: { path: { group_path: groupPath } },
          body,
          signal: options?.signal,
        }),
      ) as Promise<TaskRef>;
    },
    updateGitRepositorySettings(
      groupPath: string,
      repositoryKey: string,
      body: GitRepositorySettingsUpdateRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.PATCH("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          body,
          signal: options?.signal,
        }),
      ) as Promise<GitRepositorySource>;
    },
    indexGitRepository(
      groupPath: string,
      repositoryKey: string,
      options: RequestOptions & { forceRebuild?: boolean } = {},
    ) {
      const body: GitIndexRequest = { force_rebuild: options.forceRebuild ?? false };
      return unwrapResponse(
        openapiClient.POST("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/index", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          body,
          signal: options.signal,
        }),
      ) as Promise<TaskRef>;
    },
    setGitRepositoryConnection(
      groupPath: string,
      repositoryKey: string,
      body: GitRepositoryConnectionRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.PUT("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/connection", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          body,
          signal: options?.signal,
        }),
      ) as Promise<GitRepositorySource>;
    },
    deleteGitRepositoryConnection(groupPath: string, repositoryKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/connection", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          signal: options?.signal,
        }),
      ) as Promise<GitRepositorySource>;
    },
    listGitRepositoryFiles(
      groupPath: string,
      repositoryKey: string,
      page: GitRepositoryFilePageOptions = {},
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/files", {
          params: {
            path: { group_path: groupPath, repository_key: repositoryKey },
            query: { limit: page.limit, cursor: page.cursor },
          },
          signal: options?.signal,
        }),
      ) as Promise<GitRepositoryFileListResponse>;
    },
    getGitRepositoryFile(
      groupPath: string,
      repositoryKey: string,
      path: string,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/file", {
          params: {
            path: { group_path: groupPath, repository_key: repositoryKey },
            query: { path },
          },
          signal: options?.signal,
        }),
      ) as Promise<GitRepositoryFileDetailResponse>;
    },
    getGitRepositoryFileContent(
      groupPath: string,
      repositoryKey: string,
      content: GitRepositoryContentOptions,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.GET(
          "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/file/content",
          {
            params: {
              path: { group_path: groupPath, repository_key: repositoryKey },
              query: {
                path: content.path,
                start_line: content.startLine,
                end_line: content.endLine,
                cursor: content.cursor,
              },
            },
            signal: options?.signal,
          },
        ),
      ) as Promise<GitRepositoryFileContentResponse>;
    },
    searchGitRepositoryCode(
      groupPath: string,
      repositoryKey: string,
      search: GitRepositoryCodeSearchOptions,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.GET(
          "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/code-search",
          {
            params: {
              path: { group_path: groupPath, repository_key: repositoryKey },
              query: {
                query: search.query,
                path_prefix: search.pathPrefix,
                language: search.language,
                limit: search.limit,
              },
            },
            signal: options?.signal,
          },
        ),
      ) as Promise<GitCodeSearchResponse>;
    },
  };
}
