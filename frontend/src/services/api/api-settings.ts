import type {
  RequestOptions,
  TestRuntimeValkeyRequest,
  UpdateDoclingSettingsRequest,
  UpdateRuntimeSettingsRequest,
  UpdateRuntimeS3Settings,
  UpdateSearchSettingsRequest,
  UpdateTranslationSettingsRequest,
  TranslationProviderPageResponse,
  TaskRef,
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
    testS3Connection(payload: UpdateRuntimeS3Settings, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/settings/runtime/s3/test", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    testValkeyConnection(payload: TestRuntimeValkeyRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/settings/runtime/valkey/test", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    submitVectorIndexRebuild(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/settings/runtime/vector-index/rebuild", {
          signal: options?.signal,
        }),
      ) as Promise<TaskRef>;
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
    getTranslationSettings(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/settings/translation", { signal: options?.signal }),
      );
    },
    listTranslationProviders(params: { page: number; pageSize: number }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/settings/translation/providers", {
        params: { query: { page: params.page, page_size: params.pageSize } },
        signal: options?.signal,
      })) as Promise<TranslationProviderPageResponse>;
    },
    updateTranslationSettings(payload: UpdateTranslationSettingsRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/settings/translation", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
  };
}
