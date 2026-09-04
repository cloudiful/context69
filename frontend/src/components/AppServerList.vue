<script setup lang="ts">
import { useI18n } from "vue-i18n";

import type { Pagination } from "../services/api";
import AsyncStateBlock from "./AsyncStateBlock.vue";
import TablePagination from "./TablePagination.vue";

withDefaults(defineProps<{
  empty?: boolean;
  emptyMessage?: string;
  error?: string | null;
  loading?: boolean;
  loadingTitle?: string;
  pagination?: Pagination | null;
  pageSizeOptions?: number[];
}>(), {
  empty: false,
  emptyMessage: "",
  error: null,
  loading: false,
  loadingTitle: "",
  pagination: undefined,
  pageSizeOptions: undefined,
});

const emit = defineEmits<{
  retry: [];
  "update:page": [number];
  "update:page-size": [number];
}>();

const { t } = useI18n();
</script>

<template>
  <div class="flex min-h-0 min-w-0 flex-col gap-2 overflow-hidden">
    <slot name="toolbar" />

    <AsyncStateBlock
      :loading="loading"
      :error="error"
      :empty="empty"
      :empty-message="emptyMessage"
      :loading-title="loadingTitle"
    >
      <template #error="{ error: errorMessage }">
        <div class="grid justify-items-center gap-3 py-12 text-center">
          <UAlert color="error" variant="subtle" :title="t('common.error')" :description="errorMessage || undefined" />
          <UButton color="neutral" variant="outline" icon="i-lucide-rotate-ccw" :label="t('common.retry')" @click="emit('retry')" />
        </div>
      </template>
      <div class="min-h-0 min-w-0 flex-1 overflow-hidden">
        <slot />
      </div>
    </AsyncStateBlock>

    <TablePagination
      v-if="pagination"
      :pagination="pagination"
      :page-size-options="pageSizeOptions"
      class="shrink-0"
      @update:page="emit('update:page', $event)"
      @update:page-size="emit('update:page-size', $event)"
    />
  </div>
</template>
