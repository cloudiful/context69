import { ref, toValue, type MaybeRefOrGetter } from "vue";
import { useToast } from "@nuxt/ui/composables";

import {
  apiClient,
  type LibraryFileDetailResponse,
} from "../../services/api";
import type { ExplorerEntry } from "../../types/library";
import { canReleaseFileSource, isRetryableFileStatus } from "./library-source";
import { useAppConfirm } from "../use-app-confirm";
import { useErrorToast } from "../use-error-toast";
import { createTaskSettler } from "../use-task-settling";

interface UseLibrarySourceLifecycleOptions {
  groupPath: MaybeRefOrGetter<string>;
  loadTree: () => Promise<void>;
  t: (key: string, params?: Record<string, unknown>) => string;
}

export function useLibrarySourceLifecycle({
  groupPath,
  loadTree,
  t,
}: UseLibrarySourceLifecycleOptions) {
  const confirm = useAppConfirm();
  const toast = useToast();
  const showErrorToast = useErrorToast();
  const settler = createTaskSettler(() => loadTree());
  const retryingFileIds = ref<string[]>([]);
  const releasingFileIds = ref<string[]>([]);
  const unavailableFileIds = ref<string[]>([]);

  function notifySettledFailures(results: Array<{ status: string }>, messageKey: string) {
    if (results.some((result) => result.status === "failed")) {
      showErrorToast(null, t(messageKey));
    }
  }

  function isSourceUnavailable(fileId: string) {
    return unavailableFileIds.value.includes(fileId);
  }

  function observeSourceAvailability(detail: Pick<LibraryFileDetailResponse, "file_id" | "source_available">) {
    if (detail.source_available) {
      unavailableFileIds.value = unavailableFileIds.value.filter((id) => id !== detail.file_id);
      return;
    }
    unavailableFileIds.value = [...new Set([...unavailableFileIds.value, detail.file_id])];
  }

  function canRetry(entry: ExplorerEntry) {
    return entry.kind === "file"
      && isRetryableFileStatus(entry.ingestStatus)
      && !isSourceUnavailable(entry.id);
  }

  function canReleaseSource(entry: ExplorerEntry) {
    return entry.kind === "file" && canReleaseFileSource(entry) && !isSourceUnavailable(entry.id);
  }

  function isRetrying(fileId: string) {
    return retryingFileIds.value.includes(fileId);
  }

  function isReleasing(fileId: string) {
    return releasingFileIds.value.includes(fileId);
  }

  async function retryFile(fileId: string) {
    if (retryingFileIds.value.includes(fileId) || isSourceUnavailable(fileId)) return;
    retryingFileIds.value = [...retryingFileIds.value, fileId];
    try {
      const detail = await apiClient.getGroupLibraryFile(toValue(groupPath), fileId);
      observeSourceAvailability({ file_id: fileId, source_available: detail.source_available });
      if (!detail.source_available) {
        throw new Error(t("library.sourceMissingMessage"));
      }
      const task = await apiClient.submitTask({
        kind: "retry_file_batch",
        group_path: toValue(groupPath),
        items: [{ file_id: fileId }],
      });
      toast.add({
        color: "success",
        title: t("library.retryAccepted"),
        description: t("library.retryAcceptedMessage"),
        duration: 2500,
      });
      notifySettledFailures(await settler.settle([task]), "library.retryFailed");
    } catch (error) {
      showErrorToast(error, t("library.retryFailed"));
    } finally {
      retryingFileIds.value = retryingFileIds.value.filter((id) => id !== fileId);
    }
  }

  function releaseFileSource(fileId: string, filename: string) {
    confirm.require({
      header: t("library.releaseSource"),
      message: t("library.releaseSourceConfirm", { name: filename }),
      rejectLabel: t("common.cancel"),
      acceptLabel: t("library.releaseSourceConfirmAction"),
      accept: () => { void releaseFileSourceConfirmed(fileId); },
    });
  }

  async function releaseFileSourceConfirmed(fileId: string) {
    if (releasingFileIds.value.includes(fileId)) return;
    releasingFileIds.value = [...releasingFileIds.value, fileId];
    try {
      const detail = await apiClient.releaseGroupLibraryFileSource(toValue(groupPath), fileId);
      observeSourceAvailability({ file_id: fileId, source_available: detail.source_available });
      toast.add({
        color: "success",
        title: t("library.releaseSourceSuccess"),
        description: t("library.releaseSourceSuccessMessage"),
        duration: 2500,
      });
      await loadTree();
    } catch (error) {
      showErrorToast(error, t("library.releaseSourceFailed"));
    } finally {
      releasingFileIds.value = releasingFileIds.value.filter((id) => id !== fileId);
    }
  }

  return {
    canReleaseSource,
    canRetry,
    dispose: settler.dispose,
    isReleasing,
    isRetrying,
    observeSourceAvailability,
    releaseFileSource,
    releasingFileIds,
    retryFile,
    retryingFileIds,
    unavailableFileIds,
  };
}
