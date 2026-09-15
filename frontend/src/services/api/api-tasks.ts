import type {
  CancelActiveTasksResponse,
  ClearTaskHistoryRequest,
  ClearTaskHistoryResponse,
  QuarantineStaleSubmittingRequest,
  QuarantineStaleSubmittingResponse,
  QueueDoclingRecoveryRequest,
  QueueDoclingRecoveryResponse,
  RecoverDoclingTaskRequest,
  RecoverDoclingTaskResponse,
  RequestOptions,
  RerunTaskResponse,
  TaskItemsResponse,
  TaskItemStatus,
  TaskKind,
  TaskListQuery,
  TaskListView,
  TaskPageResponse,
  TaskRef,
  TaskResponse,
  TaskRetryResponse,
  SortDirection,
  TaskSortBy,
  TaskStatus,
  TaskSubmitRequest,
} from "./api-types";

type Deps = {
  openapiClient: import("./api-core").OpenApiClient;
  unwrapResponse: <TData>(promise: Promise<{ data?: TData; error?: unknown; response: Response }>) => Promise<TData>;
};

export function createTasksApi({ openapiClient, unwrapResponse }: Deps) {
  return {
    submitTask(payload: TaskSubmitRequest, options?: RequestOptions) {
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
      view: TaskListView;
      stage?: string | null;
      waitingReason?: string | null;
      dependencyKey?: string | null;
      sortBy?: TaskSortBy | null;
      sortDirection?: SortDirection | null;
    }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/tasks", {
        params: {
          query: {
            page: params.page,
            page_size: params.pageSize,
            query: params.query || undefined,
            kind: params.kind ?? undefined,
            status: params.status ?? undefined,
            view: params.view,
            stage: params.stage || undefined,
            waiting_reason: params.waitingReason || undefined,
            dependency_key: params.dependencyKey || undefined,
            sort_by: params.sortBy ?? undefined,
            sort_direction: params.sortDirection ?? undefined,
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
    getTaskItems(taskId: string, params: { limit: number; cursor?: string; status?: TaskItemStatus | null }, options?: RequestOptions) {
      return unwrapResponse(openapiClient.GET("/v1/tasks/{task_id}/items", {
        params: {
          path: { task_id: taskId },
          query: { limit: params.limit, cursor: params.cursor, status: params.status ?? undefined },
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
    rerunTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/{task_id}/rerun", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<RerunTaskResponse>;
    },
    cancelTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/{task_id}/cancel", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<void>;
    },
    trashTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/{task_id}/trash", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<TaskResponse>;
    },
    restoreTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/{task_id}/restore", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<TaskResponse>;
    },
    deleteTask(taskId: string, options?: RequestOptions) {
      return unwrapResponse(openapiClient.DELETE("/v1/tasks/{task_id}", {
        params: { path: { task_id: taskId } },
        signal: options?.signal,
      })) as Promise<void>;
    },
    clearTaskHistory(payload: ClearTaskHistoryRequest, options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/tasks/clear", {
        body: payload,
        signal: options?.signal,
      })) as Promise<ClearTaskHistoryResponse>;
    },
    cancelActiveTasks(options?: RequestOptions) {
      return unwrapResponse(openapiClient.POST("/v1/admin/tasks/cancel-active", {
        signal: options?.signal,
      })) as Promise<CancelActiveTasksResponse>;
    },
    recoverDoclingTask(
      taskId: string,
      payload: RecoverDoclingTaskRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(openapiClient.POST("/v1/admin/tasks/{task_id}/recover", {
        params: { path: { task_id: taskId } },
        body: payload,
        signal: options?.signal,
      })) as Promise<RecoverDoclingTaskResponse>;
    },
    queueDoclingRecovery(
      taskId: string,
      payload: QueueDoclingRecoveryRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(openapiClient.POST("/v1/admin/tasks/{task_id}/recover/queue", {
        params: { path: { task_id: taskId } },
        body: payload,
        signal: options?.signal,
      })) as Promise<QueueDoclingRecoveryResponse>;
    },
    quarantineStaleSubmitting(
      payload: QuarantineStaleSubmittingRequest,
      options?: RequestOptions,
    ) {
      return unwrapResponse(openapiClient.POST("/v1/admin/tasks/quarantine-submitting", {
        body: payload,
        signal: options?.signal,
      })) as Promise<QuarantineStaleSubmittingResponse>;
    },
  };
}
