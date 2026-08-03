import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import { apiClient, type TaskKind, type TaskPageResponse, type TaskResponse, type TaskStatus } from "../services/api";
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
  t: (key: string) => string;
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
  const kindFilter = ref<TaskKind | null>(null);
  const stageFilter = ref<string | null>(null);
  const waitingReasonFilter = ref<string | null>(null);
  const actionTaskIds = ref<string[]>([]);
  const bulkAction = ref<"retry" | "cancel" | null>(null);
  let requestController: AbortController | null = null;
  let requestId = 0;

  const isRetryableTask = (task: TaskResponse) =>
    task.status === "failed" || (task.status === "cancelled" && task.progress.failed > 0);
  const failedCount = computed(() => items.value.filter(isRetryableTask).length);
  const activeCount = computed(() => items.value.filter((task) => ["queued", "running", "waiting"].includes(task.status)).length);

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

  function isActing(task: TaskResponse) {
    return actionTaskIds.value.includes(task.task_id);
  }

  async function retryTask(task: TaskResponse) {
    if (!isRetryableTask(task) || isActing(task)) return;
    actionTaskIds.value = [...actionTaskIds.value, task.task_id];
    try {
      await apiClient.retryTask(task.task_id);
      await load();
      toast.add({ color: "success", title: t("processingQueue.retryAccepted"), description: task.task_id, duration: 2500 });
    } catch (retryError) {
      showErrorToast(retryError, t("processingQueue.retryFailed"));
    } finally {
      actionTaskIds.value = actionTaskIds.value.filter((id) => id !== task.task_id);
    }
  }

  async function cancelTask(task: TaskResponse) {
    if (!["queued", "running", "waiting"].includes(task.status) || isActing(task)) return;
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

  async function retryAllFailed() {
    if (failedCount.value === 0 || bulkAction.value) return;
    bulkAction.value = "retry";
    try {
      await Promise.all(items.value.filter(isRetryableTask).map((task) => apiClient.retryTask(task.task_id)));
      await load();
      toast.add({ color: "success", title: t("processingQueue.bulkCompleted"), duration: 3000 });
    } catch (retryError) {
      showErrorToast(retryError, t("processingQueue.bulkRetryFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  async function cancelActive() {
    if (activeCount.value === 0 || bulkAction.value) return;
    bulkAction.value = "cancel";
    try {
      await Promise.all(items.value.filter((task) => ["queued", "running", "waiting"].includes(task.status)).map((task) => apiClient.cancelTask(task.task_id)));
      await load();
      toast.add({ color: "success", title: t("processingQueue.bulkCompleted"), duration: 3000 });
    } catch (cancelError) {
      showErrorToast(cancelError, t("processingQueue.bulkCancelFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  function confirmRetryAllFailed() {
    if (failedCount.value === 0 || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.retryAll"),
      message: t("processingQueue.retryAllConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.retryAllAction"),
      accept: () => void retryAllFailed(),
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
    statusFilter,
    kindFilter,
    stageFilter,
    waitingReasonFilter,
    actionTaskIds,
    bulkAction,
    failedCount,
    activeCount,
    isRetryableTask,
    load,
    refresh: () => load(),
    submitSearch,
    setStatusFilter: (value: TaskStatus | null) => setFilter(statusFilter, value),
    setKindFilter: (value: TaskKind | null) => setFilter(kindFilter, value),
    setStageFilter: (value: string | null) => setFilter(stageFilter, value),
    setWaitingReasonFilter: (value: string | null) => setFilter(waitingReasonFilter, value),
    changePage,
    changePageSize,
    retryTask,
    cancelTask,
    isActing,
    retryAllFailed,
    cancelActive,
    confirmRetryAllFailed,
    confirmCancelActive,
  };
}
