<script setup lang="ts">
import { useRouter } from "vue-router";
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

const router = useRouter();
const { t } = useI18n();

function buildTarget(hit: SearchHit) {
  if (hit.is_library_file && hit.library_file_id) {
    return {
      name: "library",
      query: {
        file: hit.library_file_id,
      },
    };
  }

  return {
    name: "document",
    params: {
      id: hit.document_id,
    },
  };
}

function openHit(hit: SearchHit) {
  const target = buildTarget(hit);
  emit("open", hit);
  if (!router.hasRoute(String(target.name))) {
    return;
  }
  void router.push(target);
}
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
      class="search-results-table tool-table-desktop"
      @update:selection="emit('select', $event)"
      @row-click="emit('select', $event.data)"
      @row-dblclick="openHit($event.data)"
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
                <Tag class="tool-chip" :value="hit.source_key" severity="secondary" />
                <Tag class="tool-chip" :value="hit.external_id" severity="secondary" />
                <Tag class="tool-chip" :value="formatScore(hit.score)" severity="secondary" />
              </div>
            </div>
            <p v-if="hit.library_path" class="app-record-header-note">
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
            @click.stop="openHit(hit)"
          />
        </template>
      </Column>
    </DataTable>

    <div class="tool-card-list search-card-list">
      <article
        v-for="hit in hits"
        :key="hit.chunk_id"
        class="tool-card"
        :class="{ 'tool-card-selected': selectedHit?.chunk_id === hit.chunk_id }"
        @click="emit('select', hit)"
        @dblclick="openHit(hit)"
      >
        <div class="tool-card-header">
          <div class="min-w-0">
            <button
              class="min-w-0 flex-1 text-left text-sm font-semibold leading-6 text-app-text transition hover:text-app-text-muted [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
              type="button"
              @click.stop="emit('select', hit)"
            >
              {{ hit.title }}
            </button>
            <div class="tool-chip-row">
              <Tag class="tool-chip" :value="hit.source_key" severity="secondary" />
              <Tag class="tool-chip" :value="formatScore(hit.score)" severity="secondary" />
            </div>
          </div>
          <Button
            class="tool-card-open"
            data-testid="search-result-open"
            severity="secondary"
            variant="outlined"
            size="small"
            :label="t('common.open')"
            @click.stop="openHit(hit)"
          />
        </div>

        <p v-if="hit.library_path" class="tool-card-subtitle">
          {{ hit.library_path }}<span v-if="hit.library_section_label"> · {{ hit.library_section_label }}</span>
        </p>
        <p class="tool-card-snippet">{{ hit.chunk_text }}</p>
        <dl class="tool-meta-grid">
          <div>
            <dt>{{ t("search.result.published", { date: "" }).trim() }}</dt>
            <dd>{{ formatDate(hit.published_at) }}</dd>
          </div>
          <div>
            <dt>{{ t("search.result.score") }}</dt>
            <dd>{{ formatScore(hit.score) }}</dd>
          </div>
        </dl>
      </article>
    </div>
  </div>
</template>
