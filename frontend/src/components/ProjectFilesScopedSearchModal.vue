<script setup lang="ts">
import { useI18n } from "vue-i18n";

import MarkdownChunk from "./MarkdownChunk.vue";
import type { SearchHit } from "../services/api";

const props = defineProps<{
  open: boolean;
  title: string;
  loading: boolean;
  query: string;
  results: SearchHit[];
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
}>();

const { t } = useI18n();
</script>

<template>
  <UModal
    :open="props.open"
    :title="props.title"
    class="w-[min(96vw,72rem)] max-w-[min(96vw,72rem)]"
    @update:open="emit('update:open', $event)"
  >
    <template #body>
      <div class="grid min-w-0 gap-3">
        <div v-if="props.loading" class="flex flex-col items-center justify-center gap-3 py-12 text-center">
          <UIcon name="i-lucide-loader-circle" class="h-8 w-8 animate-spin text-muted" />
          <p class="text-sm text-muted">{{ t("search.scoped.searching") }}</p>
        </div>
        <UAlert
          v-else-if="props.results.length === 0"
          variant="subtle"
          :title="t('search.scoped.noResultsTitle')"
          :description="t('search.scoped.noResultsMessage')"
        />
        <ul
          v-else
          data-testid="scoped-content-search-results"
          class="grid min-h-0 min-w-0 gap-3 overflow-y-auto"
        >
          <li
            v-for="hit in props.results"
            :key="hit.chunk_id"
            class="min-w-0 rounded-md border border-default p-3"
          >
            <div class="flex min-w-0 items-start justify-between gap-2">
              <p class="min-w-0 truncate text-sm font-semibold text-color" :title="hit.title">{{ hit.title }}</p>
              <span class="shrink-0 text-xs text-muted">{{ hit.group_path }}</span>
            </div>
            <div class="mt-2 min-w-0">
              <MarkdownChunk :content="hit.chunk_text" markdown :highlight="props.query" />
            </div>
          </li>
        </ul>
      </div>
    </template>
  </UModal>
</template>
