import type {
  GitRepositoryConnectionRequest,
  GitRepositoryRegistrationRequest,
  GitRepositorySource,
  RequestOptions,
  TaskRef,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
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
    indexGitRepository(groupPath: string, repositoryKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/index", {
          params: { path: { group_path: groupPath, repository_key: repositoryKey } },
          signal: options?.signal,
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
  };
}
