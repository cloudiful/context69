<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import SearchForm from "../components/SearchForm.vue";
import SearchResultList from "../components/SearchResultList.vue";
import SearchSelectionPreview from "../components/SearchSelectionPreview.vue";
import { apiClient, type SearchHit, type SearchResponse, type SourceStatus } from "../services/api";
import { resolveSearchErrorMessage } from "../utils/search-errors";
import { addSearchHistoryEntry, readSearchHistory, type SearchHistoryEntry } from "../utils/search-history";
import {
  buildSearchPayload,
  createDefaultFilters,
  filtersFromQuery,
  filtersToQuery,
  normalizeSearchFilters,
  pageFromQuery,
  sameSearchFilters,
  saveSearchSession,
} from "../utils/search";
import { buildSearchTarget } from "../utils/search-target";
import { useErrorToast } from "../composables/use-error-toast";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();

const filters = ref(createDefaultFilters());
const results = ref<SearchResponse | null>(null);
const sources = ref<SourceStatus[]>([]);
const loading = ref(false);
const searchPage = ref(1);
const searched = ref(false);
const selectedHit = ref<SearchHit | null>(null);
const historyEntries = ref<SearchHistoryEntry[]>([]);
const showResultsPanel = computed(() => loading.value || searched.value);
const visibleHistoryEntries = computed(() => historyEntries.value.slice(0, 8));
// Lower-bound totals must not be presented as exact counts.
const resultsSummary = computed(() => {
  if (!results.value) return "";
  const pagination = results.value.pagination;
  if (pagination.total_is_exact === false) {
    return t("search.resultsAtLeast", { count: pagination.total });
  }
  return `${t("search.workspace.resultsLabel")}: ${pagination.total}`;
});

let controller: AbortController | null = null;
let searchSequence = 0;
let activeSequence = 0;
let isRouteSyncing = false;
let elapsedTimer: ReturnType<typeof setInterval> | null = null;
const elapsedLabel = ref("0.0s");
const searchStartAt = ref<number | null>(null);

function loadHistory() {
  historyEntries.value = readSearchHistory();
}

async function loadSources() {
  try {
    const response = await apiClient.listSources({ page: 1, pageSize: 100, query: "" });
    sources.value = response.items;
  } catch (error) {
    showErrorToast(error, t("search.sourceLoadFailed"));
  }
}

function startElapsed() {
  searchStartAt.value = Date.now();
  elapsedLabel.value = "0.0s";
  if (elapsedTimer) clearInterval(elapsedTimer);
  elapsedTimer = setInterval(() => {
    if (searchStartAt.value != null) {
      const diff = Date.now() - searchStartAt.value;
      elapsedLabel.value = `${(diff / 1000).toFixed(1)}s`;
    }
  }, 200);
}

function stopElapsed() {
  if (elapsedTimer) clearInterval(elapsedTimer);
  elapsedTimer = null;
}

function cancelSearch() {
  controller?.abort();
}

async function runSearch(options: { persistHistory?: boolean; page?: number; syncRoute?: boolean } = {}) {
  const targetPage = options.page ?? searchPage.value ?? 1;
  searchPage.value = targetPage;
  const payload = buildSearchPayload(filters.value, targetPage);

  if (!payload.query) {
    showErrorToast(null, t("search.emptyQuery"));
    results.value = null;
    searched.value = false;
    selectedHit.value = null;
    return;
  }

  const seq = ++searchSequence;
  activeSequence = seq;
  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  searched.value = true;
  startElapsed();

  const doSync = options.syncRoute !== false;
  if (doSync) {
    isRouteSyncing = true;
    try {
      await router.replace({
        name: "search",
        query: filtersToQuery(filters.value, targetPage),
      });
    } catch {
      // ignore navigation errors
    } finally {
      await nextTick();
      isRouteSyncing = false;
    }
  }

  if (options.persistHistory) {
    historyEntries.value = addSearchHistoryEntry(filters.value);
  }

  try {
    const nextResults = await apiClient.search(payload, { signal: controller.signal });
    if (seq !== activeSequence) return;
    results.value = nextResults;
    selectedHit.value = nextResults.items[0] ?? null;
    saveSearchSession(filters.value, targetPage);
  } catch (error) {
    if (seq !== activeSequence) return;
    if (error instanceof Error && error.name === "AbortError") {
      return;
    }
    results.value = null;
    selectedHit.value = null;
    showErrorToast(null, resolveSearchErrorMessage(error, t));
  } finally {
    if (seq === activeSequence) {
      loading.value = false;
      stopElapsed();
    }
  }
}

function updateFilters(nextFilters: typeof filters.value) {
  filters.value = nextFilters;
}

async function rerunHistory(entry: SearchHistoryEntry) {
  const normalized = normalizeSearchFilters(entry);
  filters.value = normalized;
  await runSearch({ persistHistory: true, page: 1 });
}

function handlePage(page: number) {
  void runSearch({ page });
}

function handlePageSize(size: number) {
  const normalized = Math.min(Math.max(size, 1), 50);
  filters.value = { ...filters.value, limit: normalized };
  void runSearch({ page: 1 });
}

function openHit(hit: SearchHit) {
  saveSearchSession(filters.value, searchPage.value);
  void router.push(buildSearchTarget(hit));
}

watch(
  () => route.query,
  async (newQuery) => {
    if (isRouteSyncing) return;
    const nextFilters = filtersFromQuery(newQuery);
    const nextPage = pageFromQuery(newQuery);
    const sameFilters = sameSearchFilters(filters.value, nextFilters);
    if (sameFilters && nextPage === searchPage.value) return;
    if (!nextFilters.query) {
      filters.value = nextFilters;
      searchPage.value = nextPage;
      results.value = null;
      searched.value = false;
      selectedHit.value = null;
      return;
    }
    filters.value = nextFilters;
    await runSearch({ page: nextPage, syncRoute: false });
  },
);

onMounted(async () => {
  loadHistory();
  await loadSources();
  const initialFilters = filtersFromQuery(route.query);
  const initialPage = pageFromQuery(route.query);
  filters.value = initialFilters;
  searchPage.value = initialPage;
  if (initialFilters.query) {
    await runSearch({ page: initialPage, syncRoute: false });
  }
});

onBeforeUnmount(() => {
  controller?.abort();
  stopElapsed();
});
</script>

<template>
  <div class="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden">
    <section class="min-w-0 shrink-0 overflow-hidden py-1">
      <SearchForm
        :filters="filters"
        :sources="sources"
        :busy="loading"
        :history-entries="visibleHistoryEntries"
        @history-select="rerunHistory"
        @submit="runSearch({ persistHistory: true, page: 1 })"
        @update:filters="updateFilters"
      />
    </section>

    <UCard
      v-if="showResultsPanel"
      class="search-results-panel flex min-h-0 flex-1 flex-col overflow-hidden"
      :ui="{ body: 'min-h-0 flex-1 overflow-hidden p-0 sm:p-0' }"
    >
      <template #header>
        <div class="flex items-center justify-between gap-2">
          <h2 class="text-base font-semibold text-color">{{ t("search.resultsTitle") }}</h2>
          <span v-if="results" class="text-xs text-muted">{{ resultsSummary }}</span>
        </div>
      </template>
      <AsyncStateBlock
        :loading="loading"
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
            <UButton size="sm" color="neutral" variant="ghost" @click="cancelSearch">
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

        <div v-if="results" class="grid min-h-0 flex-1 gap-3 overflow-hidden p-3 xl:grid-cols-[minmax(0,1fr)_minmax(18rem,26rem)] xl:items-start">
          <SearchResultList
            class="min-w-0 overflow-hidden"
            :hits="results.items"
            :pagination="results.pagination"
            :selected-hit="selectedHit"
            @open="openHit"
            @page="handlePage"
            @page-size="handlePageSize"
            @select="selectedHit = $event"
          />

          <SearchSelectionPreview
            class="hidden min-w-0 xl:block"
            :selected-hit="selectedHit"
            @open="openHit"
          />
        </div>
      </AsyncStateBlock>
    </UCard>
  </div>
</template>
