import type {
  GitConnectionReadinessResponse,
  GitProviderConnection,
  GitProviderConnectionRequest,
  GitWebhookRegistration,
  GitWebhookRegistrationRequest,
  RequestOptions,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createGitConnectionsApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listGitConnections(groupPath: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/git-connections", {
          params: { path: { group_path: groupPath } },
          signal: options?.signal,
        }),
      ) as Promise<GitProviderConnection[]>;
    },
    getGitConnectionReadiness(groupPath: string, connectionKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/git-connections/{connection_key}", {
          params: { path: { group_path: groupPath, connection_key: connectionKey } },
          signal: options?.signal,
        }),
      ) as Promise<GitConnectionReadinessResponse>;
    },
    createGitConnection(
      groupPath: string,
      connectionKey: string,
      body: GitProviderConnectionRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.PUT("/v1/groups/by-path/{group_path}/git-connections/{connection_key}", {
          params: { path: { group_path: groupPath, connection_key: connectionKey } },
          body,
          signal: options?.signal,
        }),
      ) as Promise<GitProviderConnection>;
    },
    disableGitConnection(groupPath: string, connectionKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/groups/by-path/{group_path}/git-connections/{connection_key}", {
          params: { path: { group_path: groupPath, connection_key: connectionKey } },
          signal: options?.signal,
        }),
      ) as Promise<GitProviderConnection>;
    },
    enableGitConnection(groupPath: string, connectionKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/groups/by-path/{group_path}/git-connections/{connection_key}/enable", {
          params: { path: { group_path: groupPath, connection_key: connectionKey } },
          signal: options?.signal,
        }),
      ) as Promise<GitProviderConnection>;
    },
    getGitRepositoryWebhook(groupPath: string, repositoryKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET(
          "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/webhook",
          {
            params: { path: { group_path: groupPath, repository_key: repositoryKey } },
            signal: options?.signal,
          },
        ),
      ) as Promise<GitWebhookRegistration>;
    },
    createGitWebhookRegistration(
      groupPath: string,
      repositoryKey: string,
      body: GitWebhookRegistrationRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(
        openapiClient.PUT(
          "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/webhook",
          {
            params: { path: { group_path: groupPath, repository_key: repositoryKey } },
            body,
            signal: options?.signal,
          },
        ),
      ) as Promise<GitWebhookRegistration>;
    },
  };
}
