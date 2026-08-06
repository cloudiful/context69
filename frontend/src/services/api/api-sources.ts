import type { RequestOptions, SourcePageResponse } from "./api-types";

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
  };
}
