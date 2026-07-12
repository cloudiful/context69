<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "primevue/tag";

import type { SearchHit } from "../services/api";
import { formatDate, formatScore } from "../utils/format";

defineProps<{
  hits: SearchHit[];
  selectedHit?: SearchHit | null;
}>();

const emit = defineEmits<{
  open: [SearchHit];
  select: [SearchHit];
}>();

const { t } = useI18n();
</script>

<template>
  <div data-testid="search-results-list">
    <DataTable
      :value="hits"
      :selection="selectedHit"
      data-key="chunk_id"
      selection-mode="single"
      resizable-columns
      column-resize-mode="expand"
      size="small"
      scrollable
      state-storage="local"
      state-key="context69:table:search-results:v5"
      table-class="min-w-[44rem]"
      class="min-w-0 max-w-full"
      @update:selection="emit('select', $event)"
      @row-click="emit('select', $event.data)"
      @row-dblclick="emit('open', $event.data)"
    >
      <Column :header="t('search.resultsTitle')" field="title" class="min-w-96">
        <template #body="{ data: hit }">
          <div class="grid min-w-0 gap-2">
            <div class="flex min-w-0 flex-wrap items-start gap-2.5">
              <Button
                class="min-w-0 flex-1 justify-start px-0 text-left text-sm font-semibold leading-6 [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
                type="button"
                text
                size="small"
                severity="secondary"
                @click.stop="emit('select', hit)"
              >
                {{ hit.title }}
              </Button>
              <div class="flex shrink-0 flex-wrap items-center gap-1.5">
                <Tag :value="hit.source_key" severity="secondary" />
                <Tag :value="hit.external_id" severity="secondary" />
                <Tag :value="formatScore(hit.score)" severity="secondary" />
              </div>
            </div>
            <p v-if="hit.library_path" class="break-words">
              {{ hit.library_path }}<span v-if="hit.library_section_label"> · {{ hit.library_section_label }}</span>
            </p>
            <p class="text-[0.88rem] leading-[1.55rem] text-muted-color [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">
              {{ hit.chunk_text }}
            </p>
          </div>
        </template>
      </Column>

      <Column :header="t('search.result.published', { date: '' }).trim()" class="w-[8.5rem]">
        <template #body="{ data: hit }">
          <span class="text-sm text-muted-color">{{ formatDate(hit.published_at) }}</span>
        </template>
      </Column>

      <Column :header="t('sources.table.action')" class="w-28">
        <template #body="{ data: hit }">
          <Button
            data-testid="search-result-open"
            text
            size="small"
            severity="secondary"
            @click.stop="emit('open', hit)"
          >
            {{ t("common.open") }}
          </Button>
        </template>
      </Column>
    </DataTable>

  </div>
</template>
