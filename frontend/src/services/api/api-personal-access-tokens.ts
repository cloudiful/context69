import type {
  CreatePersonalAccessTokenRequest,
  CreatePersonalAccessTokenResponse,
  PersonalAccessTokenResponse,
  PersonalAccessTokenPageResponse,
  RequestOptions,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createPersonalAccessTokensApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listPersonalAccessTokens(params: { page: number; pageSize: number } = { page: 1, pageSize: 50 }, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/auth/personal-access-tokens", {
          params: { query: { page: params.page, page_size: params.pageSize } },
          signal: options?.signal,
        }),
      ) as Promise<PersonalAccessTokenPageResponse>;
    },
    createPersonalAccessToken(payload: CreatePersonalAccessTokenRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/auth/personal-access-tokens", {
          body: payload,
          signal: options?.signal,
        }),
      ) as Promise<CreatePersonalAccessTokenResponse>;
    },
    revokePersonalAccessToken(tokenId: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/auth/personal-access-tokens/{token_id}", {
          params: {
            path: {
              token_id: tokenId,
            },
          },
          signal: options?.signal,
        }),
      );
    },
  };
}
