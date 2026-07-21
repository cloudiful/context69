import { onBeforeUnmount, onMounted, ref } from "vue";

import { apiClient, type LibraryIngestFailureStage, type LibraryIngestStatus, type LibraryProcessingJobResponse, type LibraryProcessingJobSummaryResponse } from "../services/api";
import { useAppConfirm } from "./use-app-confirm";
import { errorMessage, useErrorToast } from "./use-error-toast";
import { useToast } from "@nuxt/ui/composables";

const PAGE_SIZE = 25;
const EMPTY_SUMMARY: LibraryProcessingJobSummaryResponse = {
  can_manage: false,
  cleanupable_stuck_count: 0,
  failed_count: 0,
  pending_count: 0,
  retryable_failed_count: 0,
  running_count: 0,
  stuck_count: 0,
};

interface UseProcessingQueueOptions {
  t: (key: string) => string;
}

export function useProcessingQueue({ t }: UseProcessingQueueOptions) {
  const showErrorToast = useErrorToast();
  const toast = useToast();
  const confirm = useAppConfirm();
  const items = ref<LibraryProcessingJobResponse[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const page = ref(1);
  const total = ref(0);
  const totalPages = ref(0);
  const summary = ref<LibraryProcessingJobSummaryResponse>(EMPTY_SUMMARY);
  const searchInput = ref("");
  const query = ref("");
  const statusFilter = ref<LibraryIngestStatus | null>(null);
  const failureStageFilter = ref<LibraryIngestFailureStage | null>(null);
  const retryingJobIds = ref<string[]>([]);
  const bulkAction = ref<"retry" | "cleanup" | null>(null);
  let requestController: AbortController | null = null;
  let requestId = 0;

  async function load(options: { resetPage?: boolean } = {}) {
    if (options.resetPage) {
      page.value = 1;
    }

    requestController?.abort();
    requestController = new AbortController();
    const currentRequest = ++requestId;
    loading.value = true;
    error.value = null;

    try {
      const response = await apiClient.getLibraryProcessingJobs({
        page: page.value,
        pageSize: PAGE_SIZE,
        query: query.value,
        status: statusFilter.value,
        failureStage: failureStageFilter.value,
      }, { signal: requestController.signal });
      if (currentRequest !== requestId) return;
      items.value = response.items;
      page.value = response.page;
      total.value = response.total;
      totalPages.value = response.total_pages;
      summary.value = response.summary ?? EMPTY_SUMMARY;
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

  function setStatusFilter(value: LibraryIngestStatus | null) {
    if (statusFilter.value === value) return;
    statusFilter.value = value;
    void load({ resetPage: true });
  }

  function setFailureStageFilter(value: LibraryIngestFailureStage | null) {
    if (failureStageFilter.value === value) return;
    failureStageFilter.value = value;
    void load({ resetPage: true });
  }

  function changePage(value: number) {
    if (page.value === value) return;
    page.value = value;
    void load();
  }

  function replaceRetriedItem(item: LibraryProcessingJobResponse, nextJobId: string) {
    items.value = items.value.map((current) => current.job_id === item.job_id
      ? {
          ...current,
          job_id: nextJobId,
          status: "pending",
          failure_stage: null,
          error_message: null,
          started_at: null,
          finished_at: null,
          updated_at: new Date().toISOString(),
        }
      : current);
  }

  async function retryJob(item: LibraryProcessingJobResponse) {
    if (!item.can_retry || item.status !== "failed" || retryingJobIds.value.includes(item.job_id)) return;
    retryingJobIds.value = [...retryingJobIds.value, item.job_id];

    try {
      let nextJobId = item.job_id;
      if (item.kind === "url_import") {
        const response = await apiClient.retryGroupLibraryUrlImportJob(item.group_path, item.job_id);
        nextJobId = response.import_job_id;
      } else if (item.file_id) {
        const response = await apiClient.retryGroupLibraryFile(item.group_path, item.file_id);
        nextJobId = response.job_id;
      } else {
        return;
      }

      replaceRetriedItem(item, nextJobId);
      toast.add({
        color: "success",
        title: t("processingQueue.retryAccepted"),
        description: item.filename || item.source_url || item.job_id,
        duration: 2500,
      });
    } catch (retryError) {
      showErrorToast(retryError, t("processingQueue.retryFailed"));
    } finally {
      retryingJobIds.value = retryingJobIds.value.filter((jobId) => jobId !== item.job_id);
    }
  }

  function isRetrying(item: LibraryProcessingJobResponse) {
    return retryingJobIds.value.includes(item.job_id);
  }

  async function retryAllFailed() {
    if (!summary.value.can_manage || summary.value.retryable_failed_count === 0 || bulkAction.value) return;
    bulkAction.value = "retry";
    try {
      const response = await apiClient.retryFailedLibraryProcessingJobs();
      await load();
      toast.add({
        color: "success",
        title: t("processingQueue.bulkCompleted"),
        description: t("processingQueue.accepted") + ": " + response.accepted + "; " + t("processingQueue.skipped") + ": " + response.skipped,
        duration: 3500,
      });
    } catch (retryError) {
      showErrorToast(retryError, t("processingQueue.bulkRetryFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  async function cleanupStuck() {
    if (!summary.value.can_manage || summary.value.cleanupable_stuck_count === 0 || bulkAction.value) return;
    bulkAction.value = "cleanup";
    try {
      const response = await apiClient.cleanupStuckLibraryProcessingJobs();
      await load();
      toast.add({
        color: "success",
        title: t("processingQueue.bulkCompleted"),
        description: t("processingQueue.accepted") + ": " + response.accepted + "; " + t("processingQueue.skipped") + ": " + response.skipped,
        duration: 3500,
      });
    } catch (cleanupError) {
      showErrorToast(cleanupError, t("processingQueue.bulkCleanupFailed"));
    } finally {
      bulkAction.value = null;
    }
  }

  function confirmRetryAllFailed() {
    if (!summary.value.can_manage || summary.value.retryable_failed_count === 0 || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.retryAll"),
      message: t("processingQueue.retryAllConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.retryAllAction"),
      accept: () => {
        void retryAllFailed();
      },
    });
  }

  function confirmCleanupStuck() {
    if (!summary.value.can_manage || summary.value.cleanupable_stuck_count === 0 || bulkAction.value) return;
    confirm.require({
      header: t("processingQueue.cleanupStuck"),
      message: t("processingQueue.cleanupStuckConfirm"),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("processingQueue.cleanupStuckAction"),
      accept: () => {
        void cleanupStuck();
      },
    });
  }

  onMounted(() => {
    void load();
  });

  onBeforeUnmount(() => {
    requestController?.abort();
    requestId += 1;
  });

  return {
    items,
    loading,
    error,
    page,
    pageSize: PAGE_SIZE,
    total,
    totalPages,
    summary,
    searchInput,
    query,
    statusFilter,
    failureStageFilter,
    retryingJobIds,
    bulkAction,
    load,
    refresh: () => load(),
    submitSearch,
    setStatusFilter,
    setFailureStageFilter,
    changePage,
    retryJob,
    isRetrying,
    retryAllFailed,
    cleanupStuck,
    confirmRetryAllFailed,
    confirmCleanupStuck,
  };
}
