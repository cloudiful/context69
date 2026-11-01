import type { ListDocumentResultsQuery } from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createDocumentResultsApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    listDocumentResults(
      groupPath: string,
      documentId: number,
      query: ListDocumentResultsQuery,
      options?: { signal?: AbortSignal },
    ) {
      const params: {
        producer?: string;
        result_kind?: string;
        after_id?: number;
        current_only?: boolean;
        latest_only?: boolean;
        limit?: number;
      } = {};
      if (query.producer != null) {
        params.producer = query.producer;
      }
      if (query.result_kind != null) {
        params.result_kind = query.result_kind;
      }
      if (query.after_id != null) {
        params.after_id = query.after_id;
      }
      if (query.current_only != null) {
        params.current_only = query.current_only;
      }
      if (query.latest_only != null) {
        params.latest_only = query.latest_only;
      }
      if (query.limit != null) {
        params.limit = query.limit;
      }
      return unwrapResponse(
        openapiClient.GET("/v1/groups/by-path/{group_path}/documents/{document_id}/results", {
          params: {
            path: {
              group_path: groupPath,
              document_id: documentId,
            },
            query: params,
          },
          signal: options?.signal,
        }),
      );
    },
  };
}
