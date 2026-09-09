import { computed, onBeforeUnmount, ref, toValue, type MaybeRefOrGetter } from "vue";

import { apiClient, type SearchHit } from "../../services/api";
import { useErrorToast } from "../use-error-toast";

interface Options {
  groupPath: MaybeRefOrGetter<string>;
  folderPath: MaybeRefOrGetter<string | null>;
  t: (key: string, params?: Record<string, unknown>) => string;
}

function folderPrefix(path: string | null | undefined): string | null {
  if (!path) return null;
  const normalized = path.replace(/\/+$/, "");
  if (!normalized) return null;
  return normalized;
}

function matchesFolder(hit: SearchHit, prefix: string | null): boolean {
  if (!prefix) return true;
  if (!hit.is_library_file || !hit.library_path) return false;
  return hit.library_path === prefix || hit.library_path.startsWith(`${prefix}/`);
}

const SCOPED_SEARCH_LIMIT = 20;

export function useScopedContentSearch({ groupPath, folderPath, t }: Options) {
  const showErrorToast = useErrorToast();
  const query = ref("");
  const results = ref<SearchHit[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const modalVisible = ref(false);
  let requestId = 0;
  let controller: AbortController | null = null;

  const currentFolderPath = computed(() => toValue(folderPath));

  async function run() {
    const trimmed = query.value.trim();
    if (!trimmed) {
      results.value = [];
      error.value = null;
      modalVisible.value = false;
      return;
    }

    const currentRequest = ++requestId;
    controller?.abort();
    controller = new AbortController();
    loading.value = true;
    error.value = null;

    // The backend narrows results to the whole group via group_path; the folder
    // constraint is applied client-side on library_path because /v1/search has
    // no folder_id/library_path filter.
    const prefix = folderPrefix(currentFolderPath.value);
    try {
      const response = await apiClient.search(
        {
          query: trimmed,
          group_path: toValue(groupPath) || undefined,
          limit: SCOPED_SEARCH_LIMIT,
          page: 1,
        },
        { signal: controller.signal },
      );
      if (currentRequest !== requestId) return;
      results.value = response.items.filter((hit) => matchesFolder(hit, prefix));
      if (results.value.length > 0) {
        modalVisible.value = true;
      }
    } catch (cause) {
      if (cause instanceof Error && cause.name === "AbortError") return;
      if (currentRequest !== requestId) return;
      error.value = t("search.scoped.loadFailed");
      results.value = [];
      showErrorToast(cause, error.value);
    } finally {
      if (currentRequest === requestId) loading.value = false;
    }
  }

  onBeforeUnmount(() => {
    controller?.abort();
  });

  return {
    error,
    loading,
    modalVisible,
    query,
    results,
    run,
  };
}
