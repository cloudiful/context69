import { onBeforeUnmount, ref } from "vue";

import { apiClient, type LibraryFileDetailResponse } from "../../services/api";

const POLL_INTERVAL_MS = 2000;

interface UseProjectLibraryDetailOptions {
  groupKey: string;
  projectKey: string;
  loadTree: () => Promise<void>;
  selectedFileId: { value: string | null };
  t: (key: string) => string;
}

export function useProjectLibraryDetail({ groupKey, projectKey, loadTree, selectedFileId, t }: UseProjectLibraryDetailOptions) {
  const detail = ref<LibraryFileDetailResponse | null>(null);
  const detailLoading = ref(false);
  const detailError = ref("");
  const activeSectionKey = ref("");
  const activeJobs = new Set<string>();
  let pollingTimer: number | null = null;
  let detailController: AbortController | null = null;

  async function loadDetail(fileId: string | null) {
    detailController?.abort();
    if (!fileId) {
      detail.value = null;
      detailError.value = "";
      detailLoading.value = false;
      activeSectionKey.value = "";
      return;
    }
    detailController = new AbortController();
    detailLoading.value = true;
    detailError.value = "";
    try {
      detail.value = await apiClient.getProjectLibraryFile(groupKey, projectKey, fileId, { signal: detailController.signal });
      activeSectionKey.value = detail.value.sections[0]?.section_key ?? "";
      const runningJobs = detail.value.jobs.filter((job) => job.status === "pending" || job.status === "running").map((job) => job.job_id);
      if (runningJobs.length > 0) schedulePolling(runningJobs);
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") return;
      detail.value = null;
      detailError.value = error instanceof Error ? error.message : t("library.detailLoadFailed");
    } finally {
      detailLoading.value = false;
    }
  }

  function schedulePolling(jobIds: string[]) {
    for (const jobId of jobIds) activeJobs.add(jobId);
    if (pollingTimer === null) void pollJobs();
  }

  async function pollJobs() {
    if (activeJobs.size === 0) {
      pollingTimer = null;
      return;
    }
    const jobIds = [...activeJobs];
    try {
      const jobs = await Promise.all(jobIds.map(async (jobId) => {
        try {
          return await apiClient.getProjectLibraryJob(groupKey, projectKey, jobId);
        } catch {
          return null;
        }
      }));
      for (const job of jobs) {
        if (!job) continue;
        if (job.status === "failed" || job.status === "succeeded") {
          activeJobs.delete(job.job_id);
        }
      }
      await loadTree();
      await loadDetail(selectedFileId.value);
    } finally {
      if (activeJobs.size > 0) {
        pollingTimer = window.setTimeout(() => {
          void pollJobs();
        }, POLL_INTERVAL_MS);
      } else {
        pollingTimer = null;
      }
    }
  }

  function dispose() {
    detailController?.abort();
    if (pollingTimer !== null) window.clearTimeout(pollingTimer);
    activeJobs.clear();
  }

  onBeforeUnmount(dispose);

  return {
    activeSectionKey,
    detail,
    detailError,
    detailLoading,
    dispose,
    loadDetail,
    schedulePolling,
  };
}
