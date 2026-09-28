import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import { apiClient, type ClearTaskHistoryView, type SortDirection, type TaskKind, type TaskListView, type TaskPageResponse, type TaskResponse, type TaskSortBy, type TaskStatus } from "../services/api";
import { useQueueActions } from "./queue-actions";
import { ACTIVE_STATUSES, DEFAULT_QUEUE_SORT_BY_VIEW, isRecoverableTask, isTerminalTask } from "./queue-helpers";
import { useQueueStream } from "./queue-stream";
import { useAppConfirm } from "./use-app-confirm";
import { errorMessage, useErrorToast } from "./use-error-toast";
import { emptyPagination } from "./use-server-pagination";
import { useToast } from "@nuxt/ui/composables";

const DEFAULT_PAGE_SIZE = 25;

interface UseProcessingQueueOptions {
  t: (key: string, params?: Record<string, unknown>) => string;
}

export function useProcessingQueue({ t }: UseProcessingQueueOptions) {
  const showErrorToast = useErrorToast();
  const toast = useToast();
  const confirm = useAppConfirm();
  const items = ref<TaskResponse[]>([]);
  const pagination = ref<TaskPageResponse["pagination"]>(emptyPagination(DEFAULT_PAGE_SIZE));
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
  const sort = ref<{ field: TaskSortBy; direction: SortDirection }>({ ...DEFAULT_QUEUE_SORT_BY_VIEW.processing });
  const actionTaskIds = ref<string[]>([]);
  const bulkAction = ref<"recover" | "cancel" | null>(null);
  const clearAction = ref<ClearTaskHistoryView | null>(null);
  let requestController: AbortController | null = null;
  let requestId = 0;

  // Live updates (issue 405 Task E3) live in `queue-stream.ts`: the watch-all
  // SSE stream, its 20s polling fallback, the 1s structural debounce, and the
  // hidden-tab pause/resume all operate on this composable's own `items` ref
  // and `load` function, so no queue state is duplicated.
  const {
    taskStream,
    startLiveUpdates,
    stopLiveUpdates,
    handleVisibilityChange,
    disposeLiveUpdates,
  } = useQueueStream({ items, load });

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
  // status only narrows the view and never widens it. Each view restores its
  // own default ordering, and entering completed drops the stage and
  // waiting-reason filters because that view exposes no selectors for them —
  // a hidden filter must never keep narrowing a view that cannot display it.
  // A single load keeps the switch atomic instead of firing one request per
  // filter.
  function setListView(next: { view: TaskListView }) {
    if (viewFilter.value === next.view && statusFilter.value === null) return;
    viewFilter.value = next.view;
    statusFilter.value = null;
    if (next.view === "completed") {
      stageFilter.value = null;
      waitingReasonFilter.value = null;
    }
    sort.value = { ...DEFAULT_QUEUE_SORT_BY_VIEW[next.view] };
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
    // Clearing a header sort returns to the current view's default ordering
    // instead of an unsorted request, so completed never falls back to the
    // backend creation-time default.
    const fallback = DEFAULT_QUEUE_SORT_BY_VIEW[viewFilter.value];
    if (sort.value.field === fallback.field && sort.value.direction === fallback.direction) return;
    sort.value = { ...fallback };
    page.value = 1;
    void load();
  }

  // Single-item, bulk, and clear-history actions live in `queue-actions.ts`.
  // They receive this composable's own refs and collaborators, so reentrancy
  // (`actionTaskIds`) and bulk/clear exclusion keep operating on the same refs.
  const {
    isActing,
    recoverTask,
    cancelTask,
    confirmTrashTask,
    restoreTask,
    confirmDeleteTask,
    recoverAll,
    cancelActive,
    confirmRecoverAll,
    confirmCancelActive,
    clearHistory,
    confirmClearCompleted,
    confirmClearTrash,
  } = useQueueActions({
    items,
    actionTaskIds,
    bulkAction,
    clearAction,
    statusFilter,
    recoverableCount,
    activeCount,
    failedCount,
    cancelledCount,
    load,
    toast,
    confirm,
    showErrorToast,
    t,
  });

  onMounted(() => {
    void load();
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", handleVisibilityChange);
    }
  });
  onBeforeUnmount(() => {
    disposeLiveUpdates();
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
