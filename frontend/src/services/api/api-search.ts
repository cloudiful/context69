import type { RequestOptions, SearchRequest } from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createSearchApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    search(payload: SearchRequest, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.POST("/v1/search", {
          body: payload,
          signal: options?.signal,
        }),
      );
    },
    getDocument(documentId: number, options?: RequestOptions) {
      return unwrapResponse(
        openapiClient.GET("/v1/documents/{document_id}", {
          params: {
            path: {
              document_id: documentId,
            },
          },
          signal: options?.signal,
        }),
      );
    },
  };
}
