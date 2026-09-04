<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { Pagination } from "../services/api";

const props = withDefaults(defineProps<{
  pagination: Pagination;
  pageSizeOptions?: number[];
}>(), {
  pageSizeOptions: () => [10, 25, 50, 100],
});

const emit = defineEmits<{
  "update:page": [number];
  "update:page-size": [number];
}>();

const { t } = useI18n();
const effectivePageSizeOptions = computed(() => {
  const base = props.pageSizeOptions;
  const current = props.pagination.page_size;
  if (base.includes(current)) return base;
  return [...base, current].sort((a, b) => a - b);
});
const pageSizeItems = computed(() => effectivePageSizeOptions.value.map((value) => ({ label: String(value), value })));

function updatePageSize(value: unknown) {
  const next = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(next) || next === props.pagination.page_size) return;
  emit("update:page-size", next);
}
</script>

<template>
  <div v-if="props.pagination && props.pagination.total >= 0" class="flex min-h-9 flex-wrap items-center justify-between gap-2 overflow-hidden">
    <div v-if="props.pagination.total > 0" class="flex items-center gap-2">
      <span class="text-sm text-muted-color">{{ t("common.itemsPerPage") }}</span>
      <USelect
        :model-value="props.pagination.page_size"
        :items="pageSizeItems"
        value-key="value"
        class="w-20 min-w-0"
        :aria-label="t('common.itemsPerPage')"
        @update:model-value="updatePageSize($event)"
      />
    </div>
    <span v-else class="text-sm text-muted">{{ t("common.itemsPerPage") }}</span>

    <UPagination
      v-if="props.pagination.total > props.pagination.page_size"
      :page="props.pagination.page"
      :items-per-page="props.pagination.page_size"
      :total="props.pagination.total"
      class="min-w-0"
      @update:page="emit('update:page', $event)"
    />
  </div>
</template>
