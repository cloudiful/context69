import { ref, toValue, type MaybeRefOrGetter } from "vue";
import { useToast } from "@nuxt/ui/composables";

import { apiClient, type LibraryFileDetailResponse } from "../../services/api";
import { useAppConfirm } from "../use-app-confirm";
import { useErrorToast } from "../use-error-toast";
import { createTaskSettler } from "../use-task-settling";

const COLLECT_PAGE_SIZE = 100;

interface UseLibraryRetryAllFailedOptions {
  groupPath: MaybeRefOrGetter<string>;
  folderId: MaybeRefOrGetter<string | null>;
  refresh: () => Promise<void>;
  isSourceUnavailable: (fileId: string) => boolean;
  observeSourceAvailability: (
    detail: Pick<LibraryFileDetailResponse, "file_id" | "source_available">,
  ) => void;
  t: (key: string, params?: Record<string, unknown>) => string;
}

export async function collectRecursiveFailedFileIds(
  groupPath: string,
  folderId: string | null,
): Promise<string[]> {
  const ids: string[] = [];
  let page = 1;
  for (;;) {
    const response = await apiClient.getGroupLibraryResources(groupPath, {
      folderId,
      recursive: true,
      page,
      pageSize: COLLECT_PAGE_SIZE,
      query: "",
      status: "failed",
      sortBy: "updated_at",
      sortDirection: "desc",
    });
    for (const item of response.items) {
      if (item.kind === "file") ids.push(item.id);
    }
    const total = response.pagination.total;
    if (ids.length >= total || response.items.length === 0) break;
    page += 1;
    if (page > 1000) break;
  }
  return [...new Set(ids)];
}

export function useLibraryRetryAllFailed({
  groupPath,
  folderId,
  refresh,
  isSourceUnavailable,
  observeSourceAvailability,
  t,
}: UseLibraryRetryAllFailedOptions) {
  const confirm = useAppConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const settler = createTaskSettler(() => refresh());
  const retryAllBusy = ref(false);
  const retryAllFailedCount = ref(0);

  async function loadFailedCount() {
    try {
      const response = await apiClient.getGroupLibraryResources(toValue(groupPath), {
        folderId: toValue(folderId),
        recursive: true,
        page: 1,
        pageSize: 1,
        query: "",
        status: "failed",
        sortBy: "updated_at",
        sortDirection: "desc",
      });
      retryAllFailedCount.value = response.pagination.total;
    } catch {
      retryAllFailedCount.value = 0;
    }
  }

  async function retryAllFailedConfirmed() {
    if (retryAllBusy.value) return;
    retryAllBusy.value = true;
    try {
      const collected = await collectRecursiveFailedFileIds(
        toValue(groupPath),
        toValue(folderId),
      );
      const candidates = collected.filter((id) => !isSourceUnavailable(id));
      const available: string[] = [];
      let skipped = collected.length - candidates.length;
      await Promise.all(candidates.map(async (fileId) => {
        try {
          const detail = await apiClient.getGroupLibraryFile(toValue(groupPath), fileId);
          observeSourceAvailability({ file_id: fileId, source_available: detail.source_available });
          if (detail.source_available) available.push(fileId);
          else skipped += 1;
        } catch {
          skipped += 1;
        }
      }));
      if (skipped > 0) {
        toast.add({
          color: "warning",
          title: t("library.retryAllSkipped", { count: skipped }),
          duration: 3500,
        });
      }
      if (available.length === 0) {
        await loadFailedCount();
        return;
      }
      const task = await apiClient.submitTask({
        kind: "retry_file_batch",
        group_path: toValue(groupPath),
        items: available.map((file_id) => ({ file_id })),
      });
      toast.add({
        color: "success",
        title: t("library.retryAccepted"),
        description: t("library.retryAcceptedMessage"),
        duration: 2500,
      });
      const results = await settler.settle([task]);
      if (results.some((result) => result.status === "failed")) {
        showErrorToast(null, t("library.retryFailed"));
      }
      await refresh();
      await loadFailedCount();
    } catch (error) {
      showErrorToast(error, t("library.retryFailed"));
    } finally {
      retryAllBusy.value = false;
    }
  }

  function retryAllFailed() {
    if (retryAllBusy.value || retryAllFailedCount.value === 0) return;
    confirm.require({
      header: t("library.retryAllFailed"),
      message: t("library.retryAllConfirm", { count: retryAllFailedCount.value }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("library.retryAllConfirmAction"),
      accept: () => { void retryAllFailedConfirmed(); },
    });
  }

  return {
    collectRecursiveFailedFileIds,
    dispose: settler.dispose,
    loadFailedCount,
    retryAllBusy,
    retryAllFailed,
    retryAllFailedConfirmed,
    retryAllFailedCount,
  };
}
