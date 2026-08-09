import { onBeforeUnmount, ref } from "vue";

import type { PageRequest, PaginatedResponse, RequestOptions } from "../services/api";

export type ServerListSort = {
  field: string;
  direction: "asc" | "desc";
};

export type ServerListQueryRequest = PageRequest & {
  sort?: ServerListSort;
};

export type ServerPageLoader<T> = (
  request: ServerListQueryRequest,
  options?: RequestOptions,
) => Promise<PaginatedResponse<T>>;

function emptyPagination(pageSize: number) {
  return { page: 1, page_size: pageSize, total: 0, total_pages: 0 };
}

export function useServerPagination<T>(loader: ServerPageLoader<T>, initialPageSize = 50) {
  const items = ref<T[]>([]);
  const pagination = ref(emptyPagination(initialPageSize));
  const page = ref(1);
  const pageSize = ref(initialPageSize);
  const total = ref(0);
  const totalPages = ref(0);
  const sort = ref<ServerListSort | null>(null);
  const loading = ref(false);
  const error = ref<unknown>(null);
  let requestController: AbortController | null = null;
  let requestId = 0;

  async function load(requestPage = page.value, requestPageSize = pageSize.value) {
    requestController?.abort();
    requestController = new AbortController();
    const currentRequest = ++requestId;
    loading.value = true;
    error.value = null;

    try {
      const response = await loader(
        { page: requestPage, page_size: requestPageSize, sort: sort.value ?? undefined },
        { signal: requestController.signal },
      );
      if (currentRequest !== requestId) return;
      items.value = response.items;
      pagination.value = response.pagination;
      page.value = response.pagination.page;
      pageSize.value = response.pagination.page_size;
      total.value = response.pagination.total;
      totalPages.value = response.pagination.total_pages;
    } catch (cause) {
      if (cause instanceof Error && cause.name === "AbortError") return;
      if (currentRequest !== requestId) return;
      error.value = cause;
    } finally {
      if (currentRequest === requestId) loading.value = false;
    }
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

  function changeSort(field: string, direction: "asc" | "desc") {
    if (sort.value?.field === field && sort.value?.direction === direction) return;
    sort.value = { field, direction };
    page.value = 1;
    void load();
  }

  function clearSort() {
    if (!sort.value) return;
    sort.value = null;
    page.value = 1;
    void load();
  }

  function reset() {
    requestController?.abort();
    requestId += 1;
    items.value = [];
    pagination.value = emptyPagination(pageSize.value);
    page.value = 1;
    total.value = 0;
    totalPages.value = 0;
    error.value = null;
    loading.value = false;
  }

  onBeforeUnmount(() => {
    requestController?.abort();
    requestId += 1;
  });

  return {
    changePage,
    changePageSize,
    changeSort,
    clearSort,
    error,
    items,
    load,
    loading,
    page,
    pageSize,
    pagination,
    reset,
    sort,
    total,
    totalPages,
  };
}
