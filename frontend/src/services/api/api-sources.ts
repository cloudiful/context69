import type {
  RequestOptions,
  SourceConfigInput,
  SourcePageResponse,
  UpsertSourceConnectionRequest,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createSourcesApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listSources(params: { page: number; pageSize: number; query: string } = { page: 1, pageSize: 50, query: "" }, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/sources", {
          params: {
            query: {
              page: params.page,
              page_size: params.pageSize,
              query: params.query || undefined,
            },
          },
          signal: options?.signal,
        }),
      ) as Promise<SourcePageResponse>;
    },
    listSourceConnections(options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/source-connections", {
          signal: options?.signal,
        }),
      );
    },
    createSourceConnection(payload: UpsertSourceConnectionRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/source-connections", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    updateSourceConnection(payload: UpsertSourceConnectionRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/source-connections", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    deleteSourceConnection(name: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/source-connections/{name}", {
          params: {
            path: {
              name,
            },
          },
          signal: options?.signal,
        }),
      );
    },
    createSource(payload: SourceConfigInput, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/sources", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    updateSource(sourceKey: string, payload: SourceConfigInput, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.PUT("/v1/sources/{source_key}", {
          params: {
            path: {
              source_key: sourceKey,
            },
          },
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    deleteSource(sourceKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.DELETE("/v1/sources/{source_key}", {
          params: {
            path: {
              source_key: sourceKey,
            },
          },
          signal: options?.signal,
        }),
      );
    },
    syncSource(sourceKey: string, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/sources/{source_key}/sync", {
          params: {
            path: {
              source_key: sourceKey,
            },
          },
          signal: options?.signal,
        }),
      );
    },
  };
}
