import { computed, ref, watch } from "vue";

type TableItemsRef<T> = { readonly value: readonly T[] };

export function useClientTablePagination<T>(items: TableItemsRef<T>, initialPageSize = 50) {
  const page = ref(1);
  const pageSize = ref(initialPageSize);
  const total = computed(() => items.value.length);
  const visibleItems = computed(() => {
    const start = (page.value - 1) * pageSize.value;
    return items.value.slice(start, start + pageSize.value);
  });

  watch(() => items.value, () => {
    page.value = 1;
  });

  function changePage(value: number) {
    page.value = value;
  }

  function changePageSize(value: number) {
    pageSize.value = value;
    page.value = 1;
  }

  return { changePage, changePageSize, page, pageSize, total, visibleItems };
}
