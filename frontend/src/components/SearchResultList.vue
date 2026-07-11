<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import Column from "primevue/column";
import DataTable from "primevue/datatable";
import Tag from "./AppTag.vue";

import type { SearchHit } from "../services/api";
import { compactTableActionButtonClass, searchResultOpenButtonClass } from "../ui/button-classes";
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
      state-key="context69:table:search-results:v2"
      table-class="min-w-[44rem]"
      class="search-results-table hidden md:block"
      @update:selection="emit('select', $event)"
      @row-click="emit('select', $event.data)"
      @row-dblclick="emit('open', $event.data)"
    >
      <Column :header="t('search.resultsTitle')" field="title" class="min-w-96">
        <template #body="{ data: hit }">
          <div class="grid min-w-0 gap-2">
            <div class="flex min-w-0 flex-wrap items-start gap-2.5">
              <Button
                unstyled
                class="min-w-0 flex-1 text-left text-sm font-semibold leading-6 text-app-text transition hover:text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
                type="button"
                @click.stop="emit('select', hit)"
              >
                {{ hit.title }}
              </Button>
              <div class="flex shrink-0 flex-wrap items-center gap-1.5">
                <Tag class="tool-chip" :value="hit.source_key" severity="secondary" />
                <Tag class="tool-chip" :value="hit.external_id" severity="secondary" />
                <Tag class="tool-chip" :value="formatScore(hit.score)" severity="secondary" />
              </div>
            </div>
            <p v-if="hit.library_path" class="break-words">
              {{ hit.library_path }}<span v-if="hit.library_section_label"> · {{ hit.library_section_label }}</span>
            </p>
            <p class="text-[0.88rem] leading-[1.55rem] text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">
              {{ hit.chunk_text }}
            </p>
          </div>
        </template>
      </Column>

      <Column :header="t('search.result.published', { date: '' }).trim()" class="w-[8.5rem]">
        <template #body="{ data: hit }">
          <span class="text-sm text-app-text-muted">{{ formatDate(hit.published_at) }}</span>
        </template>
      </Column>

      <Column :header="t('sources.table.action')" class="w-28">
        <template #body="{ data: hit }">
          <Button
            unstyled
            :class="compactTableActionButtonClass"
            data-testid="search-result-open"
            :label="t('common.open')"
            @click.stop="emit('open', hit)"
          />
        </template>
      </Column>
    </DataTable>

    <div class="hidden grid-cols-1 md:hidden search-card-list">
      <article
        v-for="hit in hits"
        :key="hit.chunk_id"
        class="grid gap-[0.45rem] border-b border-app-border/70 bg-app-surface px-3 py-[0.65rem] text-sm last:border-b-0"
        data-testid="search-result-card"
        :class="{ 'bg-[color-mix(in_srgb,var(--color-app-surface-soft)_54%,var(--color-app-surface)_46%)]': selectedHit?.chunk_id === hit.chunk_id }"
        @click="emit('select', hit)"
        @dblclick="emit('open', hit)"
      >
        <div class="flex min-w-0 items-start justify-between gap-3">
          <div class="min-w-0">
            <Button
              unstyled
              class="min-w-0 flex-1 text-left text-sm font-semibold leading-6 text-app-text transition hover:text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
              type="button"
              @click.stop="emit('select', hit)"
            >
              {{ hit.title }}
            </Button>
            <div class="mt-1 flex min-w-0 flex-wrap gap-1">
              <Tag class="tool-chip" :value="hit.source_key" severity="secondary" />
              <Tag class="tool-chip" :value="formatScore(hit.score)" severity="secondary" />
            </div>
          </div>
          <Button
            unstyled
            :class="searchResultOpenButtonClass"
            data-testid="search-result-open"
            :label="t('common.open')"
            @click.stop="emit('open', hit)"
          />
        </div>

        <p v-if="hit.library_path" class="mt-0.5 truncate text-xs leading-5 text-app-text-dim">
          {{ hit.library_path }}<span v-if="hit.library_section_label"> · {{ hit.library_section_label }}</span>
        </p>
        <p class="[display:-webkit-box] [overflow:hidden] text-[0.82rem] leading-5 text-app-text-muted [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">{{ hit.chunk_text }}</p>
        <dl class="grid grid-cols-2 gap-x-3 gap-y-[0.35rem] text-xs text-app-text-dim">
          <div class="min-w-0">
            <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ t("search.result.published", { date: "" }).trim() }}</dt>
            <dd class="mt-0.5 truncate text-app-text-muted">{{ formatDate(hit.published_at) }}</dd>
          </div>
          <div class="min-w-0">
            <dt class="text-[0.68rem] font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ t("search.result.score") }}</dt>
            <dd class="mt-0.5 truncate text-app-text-muted">{{ formatScore(hit.score) }}</dd>
          </div>
        </dl>
      </article>
    </div>
  </div>
</template>
