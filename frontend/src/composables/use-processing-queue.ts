import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import { apiClient, type TaskKind, type TaskPageResponse, type TaskResponse, type TaskSortBy, type TaskStatus } from "../services/api";
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
  const kindFilter = ref<TaskKind | null>(null);
  const stageFilter = ref<string | null>(null);
  const waitingReasonFilter = ref<string | null>(null);
  const sort = ref<{ field: TaskSortBy; direction: "asc" | "desc" } | null>(null);
  const actionTaskIds = ref<string[]>([]);
  const bulkAction = ref<"recover" | "cancel" | null>(null);
  let requestController: AbortController | null = null;
  let requestId = 0;

  const isRecoverableTask = (task: TaskResponse) =>
    task.status === "failed" || task.status === "cancelled";
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
        stage: stageFilter.value,
        waitingReason: waitingReasonFilter.value,
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

  function changeSort(field: TaskSortBy, direction: "asc" | "desc") {
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

  async function recoverTask(task: TaskResponse) {
    if (!isRecoverableTask(task) || isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      if (task.status === "cancelled") {
        await apiClient.rerunTask(task.task_id);
      } else {
        await apiClient.retryTask(task.task_id);
      }
      await load();
      toast.add({
        color: "success",
        title: t(task.status === "cancelled" ? "processingQueue.resubmitAccepted" : "processingQueue.retryAccepted"),
        description: task.task_id,
        duration: 2500,
      });
    } catch (recoverError) {
      showErrorToast(recoverError, t(task.status === "cancelled" ? "processingQueue.resubmitFailed" : "processingQueue.retryFailed"));
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

  async function submitRecovery(task: TaskResponse): Promise<void> {
    if (task.status === "cancelled") {
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
    if (recoverableCount.value === 0 || bulkAction.value) return;
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
    if (activeCount.value === 0 || bulkAction.value) return;
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
    if (recoverableCount.value === 0 || bulkAction.value) return;
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
    if (activeCount.value === 0 || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.cancelActive"),
      message: t("processingQueue.cancelActiveConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.cancelActiveAction"),
      accept: () => void cancelActive(),
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
    kindFilter,
    stageFilter,
    waitingReasonFilter,
    actionTaskIds,
    bulkAction,
    recoverableCount,
    failedCount,
    cancelledCount,
    activeCount,
    isRecoverableTask,
    load,
    refresh: () => load(),
    submitSearch,
    setStatusFilter: (value: TaskStatus | null) => setFilter(statusFilter, value),
    setKindFilter: (value: TaskKind | null) => setFilter(kindFilter, value),
    setStageFilter: (value: string | null) => setFilter(stageFilter, value),
    setWaitingReasonFilter: (value: string | null) => setFilter(waitingReasonFilter, value),
    changePage,
    changePageSize,
    changeSort,
    clearSort,
    recoverTask,
    cancelTask,
    isActing,
    recoverAll,
    cancelActive,
    confirmRecoverAll,
    confirmCancelActive,
  };
}
