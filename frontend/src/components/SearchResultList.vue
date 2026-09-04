<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { TableColumn } from "@nuxt/ui";

import type { Pagination, SearchHit } from "../services/api";
import { formatDate, formatScore } from "../utils/format";
import AppServerList from "./AppServerList.vue";

const props = defineProps<{
  hits: SearchHit[];
  pagination: Pagination;
  selectedHit?: SearchHit | null;
}>();

const emit = defineEmits<{
  open: [SearchHit];
  select: [SearchHit];
  page: [number];
  "page-size": [number];
}>();

const { t } = useI18n();
const columns = computed<TableColumn<SearchHit>[]>(() => [
  { accessorKey: "title", header: t("search.resultsTitle") },
  { id: "published", header: t("search.result.published", { date: "" }).trim() },
  { id: "actions", header: t("sources.table.action") },
]);

const pageSizeOptions = computed(() => {
  const base = [8, 16, 24, 50];
  const current = props.pagination.page_size;
  if (base.includes(current)) return base;
  return [...base, current].sort((a, b) => a - b);
});

function selectRow(_event: Event, row: { original: SearchHit }) {
  emit("select", row.original);
}

function isSelected(hit: SearchHit): boolean {
  return !!props.selectedHit && props.selectedHit.chunk_id === hit.chunk_id;
}
</script>

<template>
  <AppServerList
    data-testid="search-results-list"
    :pagination="pagination"
    :page-size-options="pageSizeOptions"
    @update:page="emit('page', $event)"
    @update:page-size="emit('page-size', $event)"
  >
    <div class="min-h-[220px] max-h-[min(56vh,620px)] min-w-0 overflow-y-auto overscroll-contain">
      <UTable
        :data="hits"
        :columns="columns"
        class="min-w-0 max-w-full overflow-hidden"
        :ui="{ td: 'align-top' }"
        @select="selectRow"
      >
      <template #title-cell="{ row }">
        <template v-if="row.original">
          <div
            class="grid min-w-0 gap-1.5 rounded-md px-2 py-1.5"
            :class="isSelected(row.original) ? 'bg-muted/40 ring-1 ring-default' : ''"
            :data-selected="isSelected(row.original) ? '' : undefined"
            data-testid="search-result-item"
          >
            <div class="flex min-w-0 flex-wrap items-start gap-2">
              <UButton
                class="min-w-0 flex-1 justify-start px-0 text-left text-sm font-semibold leading-6 [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
                type="button"
                variant="ghost"
                size="sm"
                color="neutral"
                :aria-selected="isSelected(row.original)"
                @click.stop="emit('select', row.original)"
              >
                {{ row.original.title }}
              </UButton>
              <div class="flex max-w-full shrink-0 flex-wrap items-center gap-1">
                <UBadge :label="row.original.source_key" color="neutral" variant="subtle" class="max-w-[10rem] truncate" :title="row.original.source_key" />
                <UBadge :label="row.original.external_id" color="neutral" variant="subtle" class="max-w-[10rem] truncate" :title="row.original.external_id" />
                <UBadge :label="formatScore(row.original.score)" color="neutral" variant="subtle" />
              </div>
            </div>
            <a
              v-if="row.original.source_uri"
              :href="row.original.source_uri"
              target="_blank"
              rel="noopener noreferrer"
              class="block max-w-full truncate text-xs text-muted underline decoration-dotted underline-offset-2 hover:text-color"
              :title="row.original.source_uri"
              @click.stop
            >{{ row.original.source_uri }}</a>
            <p v-if="row.original.library_path" class="min-w-0 break-words text-xs text-muted">
              {{ row.original.library_path }}<span v-if="row.original.library_section_label"> · {{ row.original.library_section_label }}</span>
            </p>
            <p class="min-w-0 break-words text-[0.88rem] leading-[1.5rem] text-muted-color [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">
              {{ row.original.chunk_text }}
            </p>
          </div>
        </template>
      </template>

      <template #published-cell="{ row }"><span class="block max-w-[7rem] truncate text-sm text-muted" :title="formatDate(row.original.published_at)">{{ formatDate(row.original.published_at) }}</span></template>

      <template #actions-cell="{ row }">
        <UButton
          data-testid="search-result-open"
          variant="ghost"
          size="sm"
          color="neutral"
          class="shrink-0"
          @click.stop="emit('open', row.original)"
        >
          {{ t("common.open") }}
        </UButton>
      </template>
      </UTable>
    </div>
  </AppServerList>
</template>
