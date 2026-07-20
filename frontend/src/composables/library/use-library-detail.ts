import { onBeforeUnmount, ref } from "vue";

import { apiClient, type LibraryFileDetailResponse } from "../../services/api";
import { useErrorToast } from "../use-error-toast";

interface UseLibraryDetailOptions {
  t: (key: string) => string;
}

export function useLibraryDetail({ t }: UseLibraryDetailOptions) {
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
      const nextDetail = await apiClient.getLibraryFile(fileId, {
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

  onBeforeUnmount(() => {
    detailController?.abort();
    requestId += 1;
  });

  return {
    activeSectionKey,
    detail,
    detailLoading,
    loadDetail,
  };
}
