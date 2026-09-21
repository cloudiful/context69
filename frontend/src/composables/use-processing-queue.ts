import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import { apiClient, type ClearTaskHistoryView, type SortDirection, type TaskKind, type TaskListView, type TaskPageResponse, type TaskResponse, type TaskSortBy, type TaskStatus } from "../services/api";
import { useAppConfirm } from "./use-app-confirm";
import { errorMessage, useErrorToast } from "./use-error-toast";
import { useTaskStream } from "./use-task-stream";
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

  // Live updates (issue 405 Task E3): a watch-all task SSE stream drives
  // refreshes instead of the fixed 20s poll. The 20s poll is retained as the
  // automatic fallback when the stream errors, is unavailable, or the client
  // cannot use cookie-based SSE (e.g. PAT clients): zero behavior loss.
  const LIVE_FALLBACK_INTERVAL_MS = 20_000;
  // Issue 408 Task F1: off-page updates (task_id not in the current page)
  // imply a structural change (new/vanished task, shifted totals) that only
  // a full load can reflect. Collapse a burst of such frames into one
  // trailing load so a 2781-item storm costs one request, not one per frame.
  const STRUCTURAL_DEBOUNCE_MS = 1_000;
  let liveActive = false;
  let liveSynced = false;
  let fallbackTimer: ReturnType<typeof setInterval> | null = null;
  let structuralTimer: ReturnType<typeof setTimeout> | null = null;

  function cancelStructuralSync() {
    if (structuralTimer) {
      clearTimeout(structuralTimer);
      structuralTimer = null;
    }
  }

  function scheduleStructuralSync() {
    if (structuralTimer) return;
    structuralTimer = setTimeout(() => {
      structuralTimer = null;
      void load();
    }, STRUCTURAL_DEBOUNCE_MS);
  }

  function stopFallbackPolling() {
    if (fallbackTimer) {
      clearInterval(fallbackTimer);
      fallbackTimer = null;
    }
  }

  function startFallbackPolling() {
    stopFallbackPolling();
    fallbackTimer = setInterval(() => {
      if (document.visibilityState === "visible") {
        void load();
      }
    }, LIVE_FALLBACK_INTERVAL_MS);
  }

  const taskStream = useTaskStream({
    // Every (re)connect delivers a snapshot first: that snapshot-triggered
    // load() is the one full sync before deltas.
    onSnapshot: () => {
      liveSynced = true;
      stopFallbackPolling();
      cancelStructuralSync();
      void load();
    },
    // Issue 408 Task F1: an update frame carries the task's current full
    // state. When its task_id is already on the current page, merge it in
    // place (status/stage/progress/counts/error stay realtime) with zero
    // requests. Otherwise it signals a structural change: collapse the burst
    // into one trailing debounced load (~1s).
    onUpdate: (task) => {
      const index = items.value.findIndex((entry) => entry.task_id === task.task_id);
      if (index >= 0) {
        items.value[index] = { ...items.value[index], ...task };
        return;
      }
      scheduleStructuralSync();
    },
    onDone: () => {
      // Watch-all subscriptions never emit `done`; ignore defensively.
    },
    onErrorFrame: () => {
      if (liveActive) startFallbackPolling();
      if (liveSynced) {
        cancelStructuralSync();
        void load();
      }
    },
    onTransportError: () => {
      if (liveActive) startFallbackPolling();
      // Resync only when the broken stream had delivered its snapshot: before
      // the first snapshot no delta could have been missed, and the mount
      // load plus the fallback poll already cover freshness.
      if (liveSynced) {
        cancelStructuralSync();
        void load();
      }
    },
  });

  // Open the watch-all stream. Safe to call when SSE is unavailable: the
  // stream reports through the fallback path and the 20s poll takes over.
  // Issue 413 Phase 3: a hidden tab pauses SSE (zero background traffic);
  // the `visibilitychange` resume reconnects for a fresh snapshot resync.
  function startLiveUpdates() {
    if (liveActive) return;
    liveActive = true;
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    taskStream.connect();
  }

  function stopLiveUpdates() {
    liveActive = false;
    liveSynced = false;
    stopFallbackPolling();
    cancelStructuralSync();
    taskStream.disconnect();
  }

  // Issue 413 Phase 3: pause the SSE stream while the tab is hidden so a
  // background tab costs zero event traffic. The 20s fallback poll already
  // skips hidden tabs; cancel any pending structural sync so no load fires
  // while hidden. On visible, reconnect: the fresh snapshot resyncs updates
  // missed while paused (the server keeps no replay) and its
  // snapshot-triggered load() is the catch-up.
  function handleVisibilityChange() {
    if (!liveActive || typeof document === "undefined") return;
    if (document.visibilityState === "hidden") {
      cancelStructuralSync();
      taskStream.disconnect();
    } else {
      taskStream.reconnect();
    }
  }

  // A failed task is retryable in place; a cancelled task has to be rerun into
  // a fresh task record because its original idempotency key stays bound. Both
  // are offered as the single queue-level recovery action.
  const isRecoverableTask = (task: TaskResponse) =>
    task.status === "failed" || task.status === "cancelled";
  const isTerminalTask = (task: TaskResponse) => TERMINAL_STATUSES.includes(task.status);
  const recoverableCount = computed(() => items.value.filter(isRecoverableTask).length);
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

  // Single-item immediate recovery. A failed task retries in place; a
  // cancelled task is rerun into a fresh queued record.
  async function recoverTask(task: TaskResponse) {
    if (!isRecoverableTask(task) || isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      let rerunTaskId: string | null = null;
      if (task.status === "cancelled") {
        const rerun = await apiClient.rerunTask(task.task_id);
        rerunTaskId = rerun.task.task_id;
      } else {
        await apiClient.retryTask(task.task_id);
      }
      // A rerun creates a new queued task record: drop a narrowing status
      // filter so the user lands on the new task instead of a stale view
      // that hides it (issue 446 bulk-rerun-no-feedback).
      if (rerunTaskId !== null && statusFilter.value !== null) {
        statusFilter.value = null;
      }
      await load();
      toast.add({
        color: "success",
        title: t(task.status === "cancelled"
          ? "processingQueue.resubmitAccepted"
          : "processingQueue.retryAccepted"),
        description: rerunTaskId ?? task.task_id,
        duration: 2500,
      });
    } catch (recoverError) {
      showErrorToast(recoverError, t(task.status === "cancelled"
        ? "processingQueue.resubmitFailed"
        : "processingQueue.retryFailed"));
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

  // Bulk recovery: a failed task is retried in place, a cancelled task is
  // rerun into a fresh task record. Returns the new task id when a cancelled
  // task was rerun, null otherwise, so the caller can jump to the new tasks.
  async function submitRecovery(task: TaskResponse): Promise<string | null> {
    if (task.status === "cancelled") {
      const rerun = await apiClient.rerunTask(task.task_id);
      return rerun.task.task_id;
    }
    await apiClient.retryTask(task.task_id);
    return null;
  }

  function summarizeResults(results: PromiseSettledResult<unknown>[], skippedMessagePattern?: RegExp): RecoverySummary {
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
      const rerunTaskIds = results.flatMap((result) =>
        result.status === "fulfilled" && result.value ? [result.value] : []);
      // Reruns create new queued task records: drop a narrowing status filter
      // so the new tasks are visible instead of stranding the user on the old
      // filtered view (issue 446 bulk-rerun-no-feedback).
      if (rerunTaskIds.length > 0 && statusFilter.value !== null) {
        statusFilter.value = null;
      }
      await load();
      toast.add({
        color: summary.failed === 0 ? "success" : "warning",
        title: t("processingQueue.bulkSummary", {
          succeeded: summary.succeeded,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
        description: rerunTaskIds.length > 0
          ? t("processingQueue.bulkRerunCreated", { count: rerunTaskIds.length })
          : undefined,
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

  onMounted(() => {
    void load();
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", handleVisibilityChange);
    }
  });
  onBeforeUnmount(() => {
    liveActive = false;
    stopFallbackPolling();
    cancelStructuralSync();
    taskStream.disconnect();
    requestController?.abort();
    requestId += 1;
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    }
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
    failedCount,
    cancelledCount,
    activeCount,
    isRecoverableTask,
    isTerminalTask,
    load,
    refresh: () => load(),
    liveStreaming: taskStream.streaming,
    liveConnected: taskStream.connected,
    liveFallback: taskStream.usingFallback,
    startLiveUpdates,
    stopLiveUpdates,
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
