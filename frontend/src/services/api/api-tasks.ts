import type {
  GenericTaskRequest,
  RequestOptions,
  TaskItemsResponse,
  TaskKind,
  TaskListQuery,
  TaskPageResponse,
  TaskRef,
  TaskRetryResponse,
  TaskStatus,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createTasksApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    submitTask(payload: GenericTaskRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks", {
        body: payload,
        signal: options?.signal,
      })) as Promise<TaskRef>;
    },
    listTasks(params: {
      page: number;
      pageSize: number;
      query?: string;
      kind?: TaskKind | null;
      status?: TaskStatus | null;
      stage?: string | null;
      waitingReason?: string | null;
      dependencyKey?: string | null;
    }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/tasks", {
        params: {
          query: {
            page: params.page,
            page_size: params.pageSize,
            query: params.query || undefined,
            kind: params.kind ?? undefined,
            status: params.status ?? undefined,
            stage: params.stage || undefined,
            waiting_reason: params.waitingReason || undefined,
            dependency_key: params.dependencyKey || undefined,
          },
        },
        signal: options?.signal,
      })) as Promise<TaskPageResponse>;
    },
    getTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/tasks/{task_id}", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      }));
    },
    getTaskItems(taskId: string, params: { limit: number; cursor?: string }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/tasks/{task_id}/items", {
        params: {
          path: { task_id: taskId },
          query: { limit: params.limit, cursor: params.cursor },
        },
        signal: options?.signal,
      })) as Promise<TaskItemsResponse>;
    },
    retryTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/{task_id}/retry", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<TaskRetryResponse>;
    },
    cancelTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/{task_id}/cancel", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<void>;
    },
  };
}
