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

function selectRow(_event: Event, row: { original: SearchHit }) {
  emit("select", row.original);
}
</script>

<template>
  <AppServerList
    data-testid="search-results-list"
    :pagination="pagination"
    @update:page="emit('page', $event)"
    @update:page-size="emit('page-size', $event)"
  >
    <UTable
      :data="hits"
      :columns="columns"
      class="min-w-0 max-w-full"
      @select="selectRow"
    >
      <template #title-cell="{ row }">
        <template v-if="row.original">
          <div class="grid min-w-0 gap-2">
            <div class="flex min-w-0 flex-wrap items-start gap-2.5">
              <UButton
                class="min-w-0 flex-1 justify-start px-0 text-left text-sm font-semibold leading-6 [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
                type="button"
      variant="ghost"                size="sm"
                color="neutral"
                @click.stop="emit('select', row.original)"
              >
                {{ row.original.title }}
              </UButton>
              <div class="flex shrink-0 flex-wrap items-center gap-1.5">
                <UBadge :label="row.original.source_key" color="neutral" variant="subtle" />
                <UBadge :label="row.original.external_id" color="neutral" variant="subtle" />
                <UBadge :label="formatScore(row.original.score)" color="neutral" variant="subtle" />
              </div>
            </div>
            <p v-if="row.original.library_path" class="break-words">
              {{ row.original.library_path }}<span v-if="row.original.library_section_label"> · {{ row.original.library_section_label }}</span>
            </p>
            <p class="text-[0.88rem] leading-[1.55rem] text-muted-color [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">
              {{ row.original.chunk_text }}
            </p>
          </div>
        </template>
      </template>

      <template #published-cell="{ row }"><span class="text-sm text-muted">{{ formatDate(row.original.published_at) }}</span></template>

      <template #actions-cell="{ row }">
          <UButton
            data-testid="search-result-open"
      variant="ghost"            size="sm"
            color="neutral"
            @click.stop="emit('open', row.original)"
          >
            {{ t("common.open") }}
          </UButton>
      </template>
    </UTable>
  </AppServerList>
</template>
