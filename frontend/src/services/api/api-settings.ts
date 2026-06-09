import type {
  RequestOptions,
  UpdateDoclingSettingsRequest,
  UpdateRuntimeSettingsRequest,
  UpdateSearchSettingsRequest,
  UpsertProviderAccountRequest,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createSettingsApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    getRuntimeSettings(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/settings/runtime", {
          signal: options?.signal,
        }),
      );
    },
    updateRuntimeSettings(payload: UpdateRuntimeSettingsRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/settings/runtime", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    listProviderAccounts(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/settings/provider-accounts", {
          signal: options?.signal,
        }),
      );
    },
    createProviderAccount(payload: UpsertProviderAccountRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/settings/provider-accounts", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    updateProviderAccount(payload: UpsertProviderAccountRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/settings/provider-accounts", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    deleteProviderAccount(accountKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/settings/provider-accounts/{account_key}", {
          params: {
            path: {
              account_key: accountKey,
            },
          },
          signal: options?.signal,
        }),
      );
    },
    getDoclingSettings(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/settings/docling", {
          signal: options?.signal,
        }),
      );
    },
    updateDoclingSettings(payload: UpdateDoclingSettingsRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/settings/docling", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    getSearchSettings(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/settings/search", {
          signal: options?.signal,
        }),
      );
    },
    updateSearchSettings(payload: UpdateSearchSettingsRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/settings/search", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
  };
}
