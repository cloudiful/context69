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
      size="small"
      scrollable
      table-style="min-width: 44rem"
      class="hidden md:block"
      @update:selection="emit('select', $event)"
      @row-click="emit('select', $event.data)"
      @row-dblclick="emit('open', $event.data)"
    >
      <Column :header="t('search.resultsTitle')" field="title" style="min-width: 24rem">
        <template #body="{ data: hit }">
          <div class="grid min-w-0 gap-2">
            <div class="flex min-w-0 flex-wrap items-start gap-2.5">
              <button
                class="min-w-0 flex-1 text-left text-sm font-semibold leading-6 text-app-text transition hover:text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
                type="button"
                @click.stop="emit('select', hit)"
              >
                {{ hit.title }}
              </button>
              <div class="flex shrink-0 flex-wrap items-center gap-1.5">
                <Tag class="max-w-48 overflow-hidden" :value="hit.source_key" severity="secondary" />
                <Tag class="max-w-48 overflow-hidden" :value="hit.external_id" severity="secondary" />
                <Tag class="max-w-40 overflow-hidden" :value="formatScore(hit.score)" severity="secondary" />
              </div>
            </div>
            <p v-if="hit.library_path" class="break-words text-xs text-app-text-dim">
              {{ hit.library_path }}<span v-if="hit.library_section_label"> · {{ hit.library_section_label }}</span>
            </p>
            <p class="text-[0.88rem] leading-[1.55rem] text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">
              {{ hit.chunk_text }}
            </p>
          </div>
        </template>
      </Column>

      <Column :header="t('search.result.published', { date: '' }).trim()" style="width: 8.5rem">
        <template #body="{ data: hit }">
          <span class="text-sm text-app-text-muted">{{ formatDate(hit.published_at) }}</span>
        </template>
      </Column>

      <Column :header="t('sources.table.action')" style="width: 7rem">
        <template #body="{ data: hit }">
          <Button
            data-testid="search-result-open"
            severity="secondary"
            variant="outlined"
            size="small"
            :label="t('common.open')"
            @click.stop="emit('open', hit)"
          />
        </template>
      </Column>
    </DataTable>

    <div class="tool-card-list search-card-list hidden max-md:grid">
      <article
        v-for="hit in hits"
        :key="hit.chunk_id"
        class="grid gap-2 border-b border-app-border/70 bg-app-surface px-3 py-2.5 text-sm last:border-b-0"
        :class="{ 'bg-[color-mix(in_srgb,var(--color-app-surface-soft)_54%,var(--color-app-surface)_46%)]': selectedHit?.chunk_id === hit.chunk_id }"
        @click="emit('select', hit)"
        @dblclick="emit('open', hit)"
      >
        <div class="flex min-w-0 items-start justify-between gap-3">
          <div class="min-w-0">
            <button
              class="min-w-0 flex-1 text-left text-sm font-semibold leading-6 text-app-text transition hover:text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
              type="button"
              @click.stop="emit('select', hit)"
            >
              {{ hit.title }}
            </button>
            <div class="mt-1 flex min-w-0 flex-wrap gap-1">
              <Tag class="max-w-48 overflow-hidden" :value="hit.source_key" severity="secondary" />
              <Tag class="max-w-40 overflow-hidden" :value="formatScore(hit.score)" severity="secondary" />
            </div>
          </div>
          <Button
            class="shrink-0"
            data-testid="search-result-open"
            severity="secondary"
            variant="outlined"
            size="small"
            :label="t('common.open')"
            @click.stop="emit('open', hit)"
          />
        </div>

        <p v-if="hit.library_path" class="mt-0.5 truncate text-xs leading-5 text-app-text-dim">
          {{ hit.library_path }}<span v-if="hit.library_section_label"> · {{ hit.library_section_label }}</span>
        </p>
        <p class="overflow-hidden text-[0.82rem] leading-[1.35rem] text-app-text-muted [display:-webkit-box] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">{{ hit.chunk_text }}</p>
        <dl class="grid grid-cols-2 gap-x-3 gap-y-1 text-xs text-app-text-dim">
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
