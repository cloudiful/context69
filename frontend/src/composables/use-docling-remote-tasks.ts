import { ref } from "vue";
import { useI18n } from "vue-i18n";

import { apiClient } from "../services/api";
import type { DoclingRemoteTask, DoclingCleanupTask } from "../services/api/api-types";
import { summarizeApiError } from "./use-error-toast";

export const DOCLING_REMOTE_TASKS_LIMIT = 20;

export function useDoclingRemoteTasks() {
  const { t } = useI18n();

  const items = ref<DoclingRemoteTask[]>([]);
  const cleanup = ref<DoclingCleanupTask[]>([]);
  const cleaning = ref(false);
  const cleanupWoken = ref<number | null>(null);
  const hasMore = ref(false);
  const offset = ref(0);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const errorStatus = ref<number | null>(null);
  const loadedOnce = ref(false);
  let loadRevision = 0;

  async function loadPage() {
    const revision = ++loadRevision;
    const requestOffset = offset.value;
    loading.value = true;
    error.value = null;
    errorStatus.value = null;
    try {
      const response = await apiClient.listDoclingRemoteTasks({
        limit: DOCLING_REMOTE_TASKS_LIMIT,
        offset: requestOffset,
      });
      if (revision !== loadRevision || requestOffset !== offset.value) {
        return;
      }
      items.value = response.items;
      cleanup.value = response.cleanup ?? [];
      error.value = response.remote_error ?? null;
      hasMore.value = response.has_more;
      loadedOnce.value = true;
    } catch (loadError) {
      if (revision !== loadRevision || requestOffset !== offset.value) {
        return;
      }
      const summary = summarizeApiError(loadError);
      errorStatus.value = summary.status;
      error.value = summary.message || t("settings.docling.remoteTasks.loadFailed");
      if (!loadedOnce.value) {
        items.value = [];
        hasMore.value = false;
      }
    } finally {
      if (revision === loadRevision && requestOffset === offset.value) {
        loading.value = false;
      }
    }
  }

  async function cleanStoppedTasks() {
    if (cleaning.value) return;
    const revision = loadRevision;
    cleaning.value = true;
    try {
      const response = await apiClient.wakeDoclingCleanup();
      if (revision === loadRevision) {
        cleanupWoken.value = response.woken;
        await loadPage();
      }
    } catch (cause) {
      if (revision === loadRevision) {
        const summary = summarizeApiError(cause);
        error.value = summary.message || t("settings.docling.remoteTasks.loadFailed");
        errorStatus.value = summary.status;
      }
    } finally {
      cleaning.value = false;
    }
  }

  async function refresh() {
    await loadPage();
  }

  async function nextPage() {
    if (!hasMore.value || loading.value) {
      return;
    }
    offset.value += DOCLING_REMOTE_TASKS_LIMIT;
    await loadPage();
  }

  async function prevPage() {
    if (offset.value === 0 || loading.value) {
      return;
    }
    offset.value = Math.max(0, offset.value - DOCLING_REMOTE_TASKS_LIMIT);
    await loadPage();
  }

  function reset() {
    loadRevision += 1;
    offset.value = 0;
    items.value = [];
    cleanup.value = [];
    cleaning.value = false;
    cleanupWoken.value = null;
    hasMore.value = false;
    error.value = null;
    errorStatus.value = null;
    loadedOnce.value = false;
    loading.value = false;
  }

  return {
    doclingCleanupItems: cleanup,
    doclingCleanupWoken: cleanupWoken,
    doclingCleaning: cleaning,
    cleanStoppedDoclingTasks: cleanStoppedTasks,
    doclingRemoteError: error,
    doclingRemoteErrorStatus: errorStatus,
    doclingRemoteHasMore: hasMore,
    doclingRemoteItems: items,
    doclingRemoteLimit: DOCLING_REMOTE_TASKS_LIMIT,
    doclingRemoteLoadedOnce: loadedOnce,
    doclingRemoteLoading: loading,
    doclingRemoteOffset: offset,
    loadDoclingRemoteTasks: loadPage,
    nextDoclingRemotePage: nextPage,
    prevDoclingRemotePage: prevPage,
    refreshDoclingRemoteTasks: refresh,
    resetDoclingRemoteTasks: reset,
  };
}

export type DoclingRemoteTasksState = ReturnType<typeof useDoclingRemoteTasks>;
