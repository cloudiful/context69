import { onBeforeUnmount, ref, toValue, watch, type MaybeRefOrGetter } from "vue";

import { apiClient, type LibraryFileDetailResponse } from "../../services/api";
import { useErrorToast } from "../use-error-toast";

interface UseProjectLibraryDetailOptions {
  groupPath: MaybeRefOrGetter<string>;
  t: (key: string) => string;
}

export function useProjectLibraryDetail({ groupPath, t }: UseProjectLibraryDetailOptions) {
  const showErrorToast = useErrorToast();
  const detail = ref<LibraryFileDetailResponse | null>(null);
  const detailLoading = ref(false);
  const activeSectionKey = ref("");
  let detailController: AbortController | null = null;
  let requestId = 0;

  async function loadDetail(fileId: string | null) {
    detailController?.abort();
    const currentRequest = ++requestId;

    if (!fileId) {
      detail.value = null;
      detailLoading.value = false;
      activeSectionKey.value = "";
      return;
    }

    detailController = new AbortController();
    detailLoading.value = true;
    try {
      const nextDetail = await apiClient.getGroupLibraryFile(toValue(groupPath), fileId, {
        signal: detailController.signal,
      });
      if (currentRequest !== requestId) return;
      detail.value = nextDetail;
      activeSectionKey.value = nextDetail.sections[0]?.section_key ?? "";
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") return;
      if (currentRequest !== requestId) return;
      detail.value = null;
      showErrorToast(error, t("library.detailLoadFailed"));
    } finally {
      if (currentRequest === requestId) detailLoading.value = false;
    }
  }

  function dispose() {
    detailController?.abort();
    requestId += 1;
    detailController = null;
  }

  watch(() => toValue(groupPath), () => {
    dispose();
    detail.value = null;
    detailLoading.value = false;
    activeSectionKey.value = "";
  });

  onBeforeUnmount(dispose);

  return {
    activeSectionKey,
    detail,
    detailLoading,
    dispose,
    loadDetail,
  };
}
