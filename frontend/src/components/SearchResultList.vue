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

function isHttpUri(value: string | null | undefined): boolean {
  if (!value) return false;
  return /^https?:\/\//i.test(value.trim());
}
</script>

<template>
  <AppServerList
    data-testid="search-results-list"
    class="h-full min-h-0"
    :pagination="pagination"
    :page-size-options="pageSizeOptions"
    @update:page="emit('page', $event)"
    @update:page-size="emit('page-size', $event)"
  >
    <div data-testid="search-results-scroll" class="h-full min-h-[220px] min-w-0 overflow-y-auto overscroll-contain">
      <UTable
        :data="hits"
        :columns="columns"
        class="min-w-0 max-w-full overflow-hidden"
        :ui="{
          tbody: 'isolate divide-y divide-default [&>tr]:data-[selectable=true]:hover:bg-transparent [&>tr]:data-[selectable=true]:focus-visible:outline-3',
          th: 'px-2 py-1.5 text-xs',
          td: 'px-2 py-1.5 align-top whitespace-normal',
        }"
        @select="selectRow"
      >
      <template #title-cell="{ row }">
        <template v-if="row.original">
          <div
            class="grid min-w-0 gap-1 rounded-md px-2 py-1"
            :class="isSelected(row.original) ? 'bg-muted/40 ring-1 ring-default' : ''"
            :data-selected="isSelected(row.original) ? '' : undefined"
            data-testid="search-result-item"
          >
            <div class="flex min-w-0 flex-wrap items-start gap-2">
              <UButton
                data-testid="search-result-select"
                class="min-w-0 flex-1 justify-start px-0 text-left text-sm font-semibold leading-6 [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
                type="button"
                variant="link"
                size="sm"
                color="neutral"
                :aria-selected="isSelected(row.original)"
                @click.stop="emit('select', row.original)"
              >
                {{ row.original.title }}
              </UButton>
              <div class="flex shrink-0 items-center">
                <UBadge :label="formatScore(row.original.score)" color="neutral" variant="subtle" />
              </div>
            </div>
            <a
              v-if="isHttpUri(row.original.source_uri)"
              :href="row.original.source_uri ?? undefined"
              target="_blank"
              rel="noopener noreferrer"
              class="block max-w-full truncate text-xs text-muted underline decoration-dotted underline-offset-2 hover:text-color"
              :title="row.original.source_uri ?? undefined"
              @click.stop
            >{{ row.original.source_uri }}</a>
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
