import type {
  GitProviderConnection,
  GitWebhookRegistration,
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
  };
}
