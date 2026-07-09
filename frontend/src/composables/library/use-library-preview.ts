import { computed, onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import type { LibraryFileDetailResponse } from "../../services/api";
import type { FolderSummary } from "../../types/library";

interface UseLibraryPreviewOptions {
  allowDockedPreview?: boolean;
  detail: Ref<LibraryFileDetailResponse | null>;
  selectedFileId: Ref<string | null>;
  selectedFolderSummary: Ref<FolderSummary | null>;
  t: (key: string) => string;
}

export function useLibraryPreview({
  allowDockedPreview = true,
  detail,
  selectedFileId,
  selectedFolderSummary,
  t,
}: UseLibraryPreviewOptions) {
  const previewDialogVisible = ref(false);
  const previewDocked = ref(false);

  let previewModeMediaQuery: MediaQueryList | null = null;
  let previewModeListener: (() => void) | null = null;

  const previewTitle = computed(() => {
    if (selectedFileId.value && detail.value) {
      return detail.value.filename;
    }
    if (selectedFolderSummary.value) {
      return selectedFolderSummary.value.name;
    }
    return t("library.previewTitle");
  });

  const showDockedPreview = computed(() => previewDocked.value && !!selectedFileId.value);

  function syncPreviewMode() {
    if (!allowDockedPreview) {
      previewDocked.value = false;
      previewDialogVisible.value = false;
      return;
    }

    if (typeof window.matchMedia === "function") {
      previewModeMediaQuery = window.matchMedia("(min-width: 1280px)");
      previewModeListener = () => {
        previewDocked.value = !!previewModeMediaQuery?.matches;
        if (previewDocked.value) {
          previewDialogVisible.value = false;
        }
      };

      previewModeListener();
      previewModeMediaQuery.addEventListener("change", previewModeListener);
    } else {
      previewDocked.value = false;
    }

    if (previewDocked.value) {
      previewDialogVisible.value = false;
    }
  }

  function revealPreview() {
    previewDialogVisible.value = !previewDocked.value;
  }

  onMounted(syncPreviewMode);

  onBeforeUnmount(() => {
    if (previewModeMediaQuery && previewModeListener) {
      previewModeMediaQuery.removeEventListener("change", previewModeListener);
    }
  });

  return {
    previewDialogVisible,
    previewDocked,
    previewTitle,
    revealPreview,
    showDockedPreview,
  };
}
