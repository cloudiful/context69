import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import { apiClient, type ClearTaskHistoryView, type SortDirection, type TaskKind, type TaskListView, type TaskPageResponse, type TaskResponse, type TaskSortBy, type TaskStatus } from "../services/api";
import { ApiError } from "../services/api/api-core";
import { useAppConfirm } from "./use-app-confirm";
import { errorMessage, useErrorToast } from "./use-error-toast";
import { useToast } from "@nuxt/ui/composables";

const DEFAULT_PAGE_SIZE = 25;
const EMPTY_PAGINATION: TaskPageResponse["pagination"] = {
  page: 1,
  page_size: DEFAULT_PAGE_SIZE,
  total: 0,
  total_pages: 0,
};

interface UseProcessingQueueOptions {
  t: (key: string, params?: Record<string, unknown>) => string;
}

interface RecoverySummary {
  succeeded: number;
  skipped: number;
  failed: number;
}

const ACTIVE_STATUSES: TaskStatus[] = ["queued", "running", "waiting"];
const TERMINAL_STATUSES: TaskStatus[] = ["succeeded", "failed", "cancelled"];

function isUncertainSubmissionError(error: unknown): boolean {
  return error instanceof ApiError
    && error.status === 409
    && /uncertain/i.test(error.message);
}

export function useProcessingQueue({ t }: UseProcessingQueueOptions) {
  const showErrorToast = useErrorToast();
  const toast = useToast();
  const confirm = useAppConfirm();
  const items = ref<TaskResponse[]>([]);
  const pagination = ref<TaskPageResponse["pagination"]>({ ...EMPTY_PAGINATION });
  const loading = ref(false);
  const error = ref<string | null>(null);
  const page = ref(1);
  const pageSize = ref(DEFAULT_PAGE_SIZE);
  const searchInput = ref("");
  const query = ref("");
  const statusFilter = ref<TaskStatus | null>(null);
  const viewFilter = ref<TaskListView>("processing");
  const kindFilter = ref<TaskKind | null>(null);
  const stageFilter = ref<string | null>(null);
  const waitingReasonFilter = ref<string | null>(null);
  const dependencyKeyFilter = ref<string | null>(null);
  const sort = ref<{ field: TaskSortBy; direction: SortDirection } | null>(null);
  const actionTaskIds = ref<string[]>([]);
  const bulkAction = ref<"recover" | "cancel" | null>(null);
  const clearAction = ref<ClearTaskHistoryView | null>(null);
  let requestController: AbortController | null = null;
  let requestId = 0;

  // Docling polling items wait on an active external job (stage=docling_poll,
  // waiting_reason=external_job) and are not manual-recovery candidates while
  // the remote job is still pending/running and its deadline has not elapsed.
  // Only failed Docling items (failure_stage docling/docling_poll) are
  // offered as recovery; waiting polls remain pollable through the normal
  // attempt-count-exempt claim path and must not contribute to the
  // recoverable count or bulk recovery action. This matches the backend
  // guard in recover_docling_item.sql which rejects active pending/running
  // external jobs.
  const isDoclingRecoveryTask = (task: TaskResponse) =>
    task.status === "failed"
    && (task.failure_stage === "docling" || task.failure_stage === "docling_poll");
  const isRecoverableTask = (task: TaskResponse) =>
    task.status === "failed" || task.status === "cancelled" || isDoclingRecoveryTask(task);
  const isTerminalTask = (task: TaskResponse) => TERMINAL_STATUSES.includes(task.status);
  const recoverableCount = computed(() => items.value.filter(isRecoverableTask).length);
  const doclingRecoveryCount = computed(() => items.value.filter(isDoclingRecoveryTask).length);
  const activeCount = computed(() => items.value.filter((task) => ACTIVE_STATUSES.includes(task.status)).length);
  const failedCount = computed(() => items.value.filter((task) => task.status === "failed").length);
  const cancelledCount = computed(() => items.value.filter((task) => task.status === "cancelled").length);

  async function load(options: { resetPage?: boolean } = {}) {
    if (options.resetPage) page.value = 1;
    requestController?.abort();
    requestController = new AbortController();
    const currentRequest = ++requestId;
    loading.value = true;
    error.value = null;

    try {
      const response = await apiClient.listTasks({
        page: page.value,
        pageSize: pageSize.value,
        query: query.value,
        kind: kindFilter.value,
        status: statusFilter.value,
        view: viewFilter.value,
        stage: stageFilter.value,
        waitingReason: waitingReasonFilter.value,
        dependencyKey: dependencyKeyFilter.value,
        sortBy: sort.value?.field,
        sortDirection: sort.value?.direction,
      }, { signal: requestController.signal });
      if (currentRequest !== requestId) return;
      items.value = response.items;
      pagination.value = response.pagination;
      page.value = response.pagination.page;
      pageSize.value = response.pagination.page_size;
    } catch (loadError) {
      if (loadError instanceof Error && loadError.name === "AbortError") return;
      if (currentRequest !== requestId) return;
      error.value = errorMessage(loadError, t("processingQueue.loadFailed"));
    } finally {
      if (currentRequest === requestId) loading.value = false;
    }
  }

  function submitSearch() {
    query.value = searchInput.value.trim();
    void load({ resetPage: true });
  }

  function setFilter<T>(target: { value: T }, value: T) {
    if (target.value === value) return;
    target.value = value;
    void load({ resetPage: true });
  }

  // Tab changes move the typed list view at once (processing/completed/trash).
  // The backend view owns the trash/succeeded predicate; a user-selected
  // status only narrows the view and never widens it. A single load keeps
  // the switch atomic instead of firing one request per filter.
  function setListView(next: { view: TaskListView }) {
    if (viewFilter.value === next.view && statusFilter.value === null) return;
    viewFilter.value = next.view;
    statusFilter.value = null;
    void load({ resetPage: true });
  }

  function changePage(value: number) {
    if (page.value === value) return;
    page.value = value;
    void load();
  }

  function changePageSize(value: number) {
    if (pageSize.value === value) return;
    pageSize.value = value;
    page.value = 1;
    void load();
  }

  function changeSort(field: TaskSortBy, direction: SortDirection) {
    if (sort.value?.field === field && sort.value?.direction === direction) return;
    sort.value = { field, direction };
    page.value = 1;
    void load();
  }

  function clearSort() {
    if (!sort.value) return;
    sort.value = null;
    page.value = 1;
    void load();
  }

  function isActing(task: TaskResponse) {
    return actionTaskIds.value.includes(task.task_id);
  }

  // Single-item immediate recovery. A 409 uncertain rejection refreshes
  // first: the row must be quarantined, never assumed remotely cancelled.
  async function recoverTask(task: TaskResponse) {
    if (!isRecoverableTask(task) || isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      if (isDoclingRecoveryTask(task)) {
        await apiClient.recoverDoclingTask(task.task_id, {
          reason: "manual recovery from the processing queue",
        });
      } else if (task.status === "cancelled") {
        await apiClient.rerunTask(task.task_id);
      } else {
        await apiClient.retryTask(task.task_id);
      }
      await load();
      toast.add({
        color: "success",
        title: t(isDoclingRecoveryTask(task)
          ? "processingQueue.doclingRecoveryAccepted"
          : task.status === "cancelled"
            ? "processingQueue.resubmitAccepted"
            : "processingQueue.retryAccepted"),
        description: task.task_id,
        duration: 2500,
      });
    } catch (recoverError) {
      if (isDoclingRecoveryTask(task) && isUncertainSubmissionError(recoverError)) {
        await load();
        showErrorToast(recoverError, t("processingQueue.uncertainRecoveryBlocked"));
      } else {
        showErrorToast(recoverError, t(isDoclingRecoveryTask(task)
          ? "processingQueue.doclingRecoveryFailed"
          : task.status === "cancelled"
            ? "processingQueue.resubmitFailed"
            : "processingQueue.retryFailed"));
      }
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  // Item-level Docling recovery. The task-level recoverTask refuses tasks
  // whose own failure_stage is not docling/docling_poll, but a per-item
  // recovery for an item with a Docling failure_stage should still route
  // through the admin endpoint. Shares the actionTaskIds guard so row and
  // cell requests for the same task cannot run concurrently.
  async function recoverDoclingFromItem(task: TaskResponse) {
    if (isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      await apiClient.recoverDoclingTask(task.task_id, {
        reason: "manual Docling recovery from item in the processing queue",
      });
      await load();
      toast.add({
        color: "success",
        title: t("processingQueue.doclingRecoveryAccepted"),
        description: task.task_id,
        duration: 2500,
      });
    } catch (recoverError) {
      if (isUncertainSubmissionError(recoverError)) {
        await load();
        showErrorToast(recoverError, t("processingQueue.uncertainRecoveryBlocked"));
      } else {
        showErrorToast(recoverError, t("processingQueue.doclingRecoveryFailed"));
      }
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  async function cancelTask(task: TaskResponse) {
    if (!ACTIVE_STATUSES.includes(task.status) || isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      await apiClient.cancelTask(task.task_id);
      await load();
      toast.add({ color: "success", title: t("processingQueue.cancelAccepted"), description: task.task_id, duration: 2500 });
    } catch (cancelError) {
      showErrorToast(cancelError, t("processingQueue.cancelFailed"));
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  // Move a terminal task's history to the recycle bin. The backend rejects
  // active tasks, so the action is only offered for terminal rows; files and
  // processed results are never touched.
  async function trashTask(task: TaskResponse) {
    if (!isTerminalTask(task) || isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      await apiClient.trashTask(task.task_id);
      await load();
      toast.add({ color: "success", title: t("processingQueue.trashAccepted"), description: task.task_id, duration: 2500 });
    } catch (trashError) {
      showErrorToast(trashError, t("processingQueue.trashFailed"));
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  function confirmTrashTask(task: TaskResponse) {
    if (!isTerminalTask(task) || isActing(task)) return;
    confirm.require({
      header: t("processingQueue.trash"),
      message: t("processingQueue.trashConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.trashAction"),
      accept: () => void trashTask(task),
    });
  }

  async function restoreTask(task: TaskResponse) {
    if (isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      await apiClient.restoreTask(task.task_id);
      await load();
      toast.add({ color: "success", title: t("processingQueue.restoreAccepted"), description: task.task_id, duration: 2500 });
    } catch (restoreError) {
      showErrorToast(restoreError, t("processingQueue.restoreFailed"));
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  // Permanent removal is only allowed from the recycle bin; the confirmation
  // states explicitly that files and processed results are unaffected.
  async function deletePermanently(task: TaskResponse) {
    if (isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      await apiClient.deleteTask(task.task_id);
      await load();
      toast.add({ color: "success", title: t("processingQueue.deleteAccepted"), description: task.task_id, duration: 2500 });
    } catch (deleteError) {
      showErrorToast(deleteError, t("processingQueue.deleteFailed"));
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  function confirmDeleteTask(task: TaskResponse) {
    if (isActing(task)) return;
    confirm.require({
      header: t("processingQueue.deletePermanently"),
      message: t("processingQueue.deletePermanentlyConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.deletePermanentlyAction"),
      accept: () => void deletePermanently(task),
    });
  }

  // Bulk Docling recovery is queue-only: park on `docling` for dispatcher
  // pickup under max_inflight, no POST or new attempt/job. `already_queued`
  // stays fulfilled and counts as succeeded. Others keep retry/rerun.
  async function submitRecovery(task: TaskResponse): Promise<void> {
    if (isDoclingRecoveryTask(task)) {
      await apiClient.queueDoclingRecovery(task.task_id, {
        reason: "bulk queue-only recovery from the processing queue",
      });
    } else if (task.status === "cancelled") {
      await apiClient.rerunTask(task.task_id);
    } else {
      await apiClient.retryTask(task.task_id);
    }
  }

  function summarizeResults(results: PromiseSettledResult<void>[], skippedMessagePattern?: RegExp): RecoverySummary {
    const summary: RecoverySummary = { succeeded: 0, skipped: 0, failed: 0 };
    for (const result of results) {
      if (result.status === "fulfilled") {
        summary.succeeded += 1;
      } else if (skippedMessagePattern && result.reason instanceof Error && skippedMessagePattern.test(result.reason.message)) {
        summary.skipped += 1;
      } else {
        summary.failed += 1;
      }
    }
    return summary;
  }

  async function recoverAll() {
    if (recoverableCount.value === 0 || bulkAction.value || clearAction.value) return;
    bulkAction.value = "recover";
    try {
      const tasks = items.value.filter(isRecoverableTask);
      const results = await Promise.allSettled(tasks.map((task) => submitRecovery(task)));
      const summary = summarizeResults(results, /no retryable/i);
      await load();
      toast.add({
        color: summary.failed === 0 ? "success" : "warning",
        title: t("processingQueue.bulkSummary", {
          succeeded: summary.succeeded,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
        duration: 3500,
      });
    } catch (recoverError) {
      showErrorToast(recoverError, t("processingQueue.bulkRetryFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  async function cancelActive() {
    if (activeCount.value === 0 || bulkAction.value || clearAction.value) return;
    bulkAction.value = "cancel";
    try {
      const results = await Promise.allSettled(items.value.filter((task) => ACTIVE_STATUSES.includes(task.status)).map((task) => apiClient.cancelTask(task.task_id)));
      const summary = summarizeResults(results);
      await load();
      toast.add({
        color: summary.failed === 0 ? "success" : "warning",
        title: t("processingQueue.bulkSummary", {
          succeeded: summary.succeeded,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
        duration: 3500,
      });
    } catch (cancelError) {
      showErrorToast(cancelError, t("processingQueue.bulkCancelFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  function confirmRecoverAll() {
    if (recoverableCount.value === 0 || bulkAction.value || clearAction.value) return;
    confirm.require({
      header: t("processingQueue.retryAll"),
      message: t("processingQueue.recoverAllConfirm", {
        failed: failedCount.value,
        cancelled: cancelledCount.value,
        docling: doclingRecoveryCount.value,
      }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.retryAllAction"),
      accept: () => void recoverAll(),
    });
  }

  function confirmCancelActive() {
    if (activeCount.value === 0 || bulkAction.value || clearAction.value) return;
    confirm.require({
      header: t("processingQueue.cancelActive"),
      message: t("processingQueue.cancelActiveConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.cancelActiveAction"),
      accept: () => void cancelActive(),
    });
  }

  // User-scoped history clearing. `completed` removes only the current
  // user's untrashed succeeded tasks; `trash` removes only the current
  // user's trashed terminal tasks. Active tasks, other users' tasks, and
  // files/documents/vectors are never touched. Repeat calls are idempotent
  // and report the actual deleted count.
  async function clearHistory(view: ClearTaskHistoryView) {
    if (clearAction.value || bulkAction.value) return;
    clearAction.value = view;
    try {
      const response = await apiClient.clearTaskHistory({ view });
      await load();
      toast.add({
        color: "success",
        title: t(view === "completed" ? "processingQueue.clearCompletedAccepted" : "processingQueue.clearTrashAccepted"),
        description: String(response.deleted_count),
        duration: 3000,
      });
    } catch (clearError) {
      showErrorToast(clearError, t(view === "completed" ? "processingQueue.clearCompletedFailed" : "processingQueue.clearTrashFailed"));
    } finally {
      clearAction.value = null;
    }
  }

  function confirmClearCompleted() {
    if (clearAction.value || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.clearCompleted"),
      message: t("processingQueue.clearCompletedConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.clearCompletedAction"),
      accept: () => void clearHistory("completed"),
    });
  }

  function confirmClearTrash() {
    if (clearAction.value || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.clearTrash"),
      message: t("processingQueue.clearTrashConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.clearTrashAction"),
      accept: () => void clearHistory("trash"),
    });
  }

  onMounted(() => void load());
  onBeforeUnmount(() => {
    requestController?.abort();
    requestId += 1;
  });

  return {
    items,
    pagination,
    loading,
    error,
    page,
    pageSize,
    searchInput,
    query,
    sort,
    statusFilter,
    viewFilter,
    kindFilter,
    stageFilter,
    waitingReasonFilter,
    dependencyKeyFilter,
    actionTaskIds,
    bulkAction,
    clearAction,
    recoverableCount,
    doclingRecoveryCount,
    failedCount,
    cancelledCount,
    activeCount,
    isRecoverableTask,
    isTerminalTask,
    isDoclingRecoveryTask,
    load,
    refresh: () => load(),
    submitSearch,
    setListView,
    setStatusFilter: (value: TaskStatus | null) => setFilter(statusFilter, value),
    setKindFilter: (value: TaskKind | null) => setFilter(kindFilter, value),
    setStageFilter: (value: string | null) => setFilter(stageFilter, value),
    setWaitingReasonFilter: (value: string | null) => setFilter(waitingReasonFilter, value),
    setDependencyKeyFilter: (value: string | null) => setFilter(dependencyKeyFilter, value),
    changePage,
    changePageSize,
    changeSort,
    clearSort,
    recoverTask,
    recoverDoclingFromItem,
    cancelTask,
    confirmTrashTask,
    restoreTask,
    confirmDeleteTask,
    isActing,
    recoverAll,
    cancelActive,
    confirmRecoverAll,
    confirmCancelActive,
    clearHistory,
    confirmClearCompleted,
    confirmClearTrash,
  };
}
