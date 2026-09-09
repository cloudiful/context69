<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "./AsyncStateBlock.vue";
import SearchResultList from "./SearchResultList.vue";
import SearchSelectionPreview from "./SearchSelectionPreview.vue";
import type { SearchHit, SearchResponse } from "../services/api";

const props = withDefaults(defineProps<{
  results: SearchResponse | null;
  loading?: boolean;
  refining?: boolean;
  fallbackNotice?: boolean;
  searched?: boolean;
  highlight?: string;
  sort?: "relevance" | "date";
}>(), {
  loading: false,
  refining: false,
  fallbackNotice: false,
  searched: false,
  highlight: "",
  sort: "relevance",
});

const emit = defineEmits<{
  open: [SearchHit];
  cancel: [];
  prev: [];
  next: [];
  "page-size": [number];
}>();

const { t } = useI18n();

const selectedHit = ref<SearchHit | null>(null);
const previewOpen = ref(false);
const previewTitle = computed(() => selectedHit.value?.title ?? "");
// Lower-bound totals must not be presented as exact counts.
const resultsSummary = computed(() => {
  if (!props.results) return "";
  const pagination = props.results.pagination;
  if (pagination.total_is_exact === false) {
    return t("search.resultsAtLeast", { count: pagination.total });
  }
  return `${t("search.workspace.resultsLabel")}: ${pagination.total}`;
});

let elapsedTimer: ReturnType<typeof setInterval> | null = null;
const elapsedLabel = ref("0.0s");
const elapsedStartAt = ref<number | null>(null);

function startElapsed() {
  elapsedStartAt.value = Date.now();
  elapsedLabel.value = "0.0s";
  if (elapsedTimer) clearInterval(elapsedTimer);
  elapsedTimer = setInterval(() => {
    if (elapsedStartAt.value != null) {
      const diff = Date.now() - elapsedStartAt.value;
      elapsedLabel.value = `${(diff / 1000).toFixed(1)}s`;
    }
  }, 200);
}

function stopElapsed() {
  if (elapsedTimer) clearInterval(elapsedTimer);
  elapsedTimer = null;
}

// The elapsed timer runs while the page is loading (spinner) and while a
// rerank is refining the already-rendered local page (slim indicator).
watch(
  () => [props.loading, props.refining],
  ([loading, refining]) => {
    if (loading || refining) {
      if (elapsedStartAt.value == null) startElapsed();
    } else {
      elapsedStartAt.value = null;
      stopElapsed();
    }
  },
  { immediate: true },
);

watch(
  () => props.results,
  () => {
    // A new page replaces the previous selection context.
    selectedHit.value = null;
    previewOpen.value = false;
  },
);

function openPreview(hit: SearchHit) {
  selectedHit.value = hit;
  previewOpen.value = true;
}

function openFromModal(hit: SearchHit) {
  previewOpen.value = false;
  emit("open", hit);
}
</script>

<template>
  <UCard
    class="search-results-panel flex min-h-0 flex-1 flex-col overflow-hidden"
    :ui="{ body: 'flex min-h-0 flex-1 flex-col overflow-hidden p-0 sm:p-0' }"
  >
    <template #header>
      <div class="flex min-w-0 items-center justify-between gap-2">
        <h2 class="text-base font-semibold text-color">{{ t("search.resultsTitle") }}</h2>
        <div class="flex min-w-0 items-center gap-2">
          <span v-if="refining" data-testid="search-refining" class="flex min-w-0 items-center gap-1.5 text-xs text-muted">
            <UIcon name="i-lucide-loader-circle" class="h-3.5 w-3.5 animate-spin" />
            <span class="truncate">{{ t("search.streaming.refining") }}</span>
            <span class="tabular-nums">{{ elapsedLabel }}</span>
          </span>
          <span v-if="results" class="text-xs text-muted">{{ resultsSummary }}</span>
        </div>
      </div>
    </template>
    <UAlert
      v-if="fallbackNotice"
      data-testid="search-stream-fallback-notice"
      variant="subtle"
      color="neutral"
      icon="i-lucide-radio-tower"
      :title="t('search.streaming.fallbackNotice')"
      class="m-2 shrink-0"
      :ui="{ inner: 'flex items-center gap-2' }"
    />
    <AsyncStateBlock
      class="min-h-0 min-w-0 flex-1"
      :loading="loading && !results"
      :loading-title="t('search.scanningTitle')"
      :loading-message="t('search.scanningMessage')"
      loading-test-id="search-loading"
      :empty="searched && !!results && results.items.length === 0"
    >
      <template #loading>
        <div class="flex flex-col items-center justify-center gap-3 py-12 text-center">
          <UIcon name="i-lucide-loader-circle" data-testid="search-loading" class="h-10 w-10 animate-spin text-muted" />
          <div class="grid gap-1">
            <p class="text-sm font-medium text-color">{{ t("search.scanningTitle") }} · {{ elapsedLabel }}</p>
            <p class="text-xs text-muted">{{ t("search.form.waiting") }}</p>
          </div>
          <UButton size="sm" color="neutral" variant="ghost" @click="emit('cancel')">
            {{ t("common.cancel") }}
          </UButton>
        </div>
      </template>
      <template #empty>
        <UAlert
          variant="subtle"
          :title="t('search.noMatchesTitle')"
          :description="t('search.noMatchesMessage')"
        />
      </template>

      <div v-if="results" class="grid min-h-0 flex-1 gap-3 overflow-hidden p-3">
        <SearchResultList
          class="h-full min-h-0 min-w-0 overflow-hidden"
          :hits="results.items"
          :pagination="results.pagination"
          :selected-hit="selectedHit"
          :highlight="highlight"
          :sort="props.sort"
          @preview="openPreview"
          @open="emit('open', $event)"
          @prev="emit('prev')"
          @next="emit('next')"
          @page-size="emit('page-size', $event)"
        />
      </div>
    </AsyncStateBlock>

    <UModal
      v-model:open="previewOpen"
      :title="previewTitle"
      class="w-[min(96vw,72rem)] max-w-[min(96vw,72rem)]"
    >
      <template #body>
        <SearchSelectionPreview
          :selected-hit="selectedHit"
          :highlight="highlight"
          @open="openFromModal"
        />
      </template>
    </UModal>
  </UCard>
</template>
