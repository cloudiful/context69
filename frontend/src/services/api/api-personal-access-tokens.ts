import type {
  CreatePersonalAccessTokenRequest,
  CreatePersonalAccessTokenResponse,
  GroupPageResponse,
  PersonalAccessTokenResponse,
  PersonalAccessTokenPageResponse,
  RequestOptions,
  RotatePersonalAccessTokenRequest,
  UpdatePersonalAccessTokenRequest,
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
    listPersonalAccessTokenGroupCatalog(
      params: { page: number; pageSize: number; query?: string } = { page: 1, pageSize: 50 },
      options?: RequestOptions,
    ) {
      const query = params.query?.trim() ?? "";
      return unwrapResponse(
        openapiClient.GET("/v1/auth/personal-access-tokens/group-catalog", {
          params: {
            query: {
              page: params.page,
              page_size: params.pageSize,
              query: query.length > 0 ? query : undefined,
            },
          },
          signal: options?.signal,
        }),
      ) as Promise<GroupPageResponse>;
    },
    updatePersonalAccessToken(tokenId: string, payload: UpdatePersonalAccessTokenRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PATCH("/v1/auth/personal-access-tokens/{token_id}", {
          params: {
            path: {
              token_id: tokenId,
            },
          },
          body: payload,
          signal: options?.signal,
        }),
      ) as Promise<PersonalAccessTokenResponse>;
    },
    rotatePersonalAccessToken(tokenId: string, payload: RotatePersonalAccessTokenRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/auth/personal-access-tokens/{token_id}/rotate", {
          params: {
            path: {
              token_id: tokenId,
            },
          },
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
