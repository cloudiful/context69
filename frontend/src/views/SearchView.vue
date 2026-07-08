<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import AppPanel from "../components/AppPanel.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import SearchForm from "../components/SearchForm.vue";
import SearchHistoryPanel from "../components/SearchHistoryPanel.vue";
import SearchResultList from "../components/SearchResultList.vue";
import SearchSelectionPreview from "../components/SearchSelectionPreview.vue";
import { apiClient, type SearchHit, type SearchResponse, type SourceStatus } from "../services/api";
import { resolveSearchErrorMessage } from "../utils/search-errors";
import { addSearchHistoryEntry, clearSearchHistory, readSearchHistory, type SearchHistoryEntry } from "../utils/search-history";
import { createDefaultFilters, filtersFromQuery, filtersToQuery, buildSearchPayload } from "../utils/search";
import { buildSearchTarget } from "../utils/search-target";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

const filters = ref(createDefaultFilters());
const results = ref<SearchResponse | null>(null);
const sources = ref<SourceStatus[]>([]);
const loading = ref(false);
const sourceError = ref("");
const searchError = ref("");
const searched = ref(false);
const selectedHit = ref<SearchHit | null>(null);
const historyEntries = ref<SearchHistoryEntry[]>([]);
const showResultsPanel = computed(() => loading.value || !!searchError.value || searched.value);
const visibleHistoryEntries = computed(() => historyEntries.value.slice(0, 8));

let controller: AbortController | null = null;

function loadHistory() {
  historyEntries.value = readSearchHistory();
}

async function loadSources() {
  try {
    sourceError.value = "";
    sources.value = await apiClient.listSources();
  } catch (error) {
    sourceError.value = error instanceof Error ? error.message : t("search.sourceLoadFailed");
  }
}

async function runSearch(options: { persistHistory?: boolean } = {}) {
  const payload = buildSearchPayload(filters.value);

  if (!payload.query) {
    searchError.value = t("search.emptyQuery");
    results.value = null;
    searched.value = false;
    return;
  }

  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  searchError.value = "";
  searched.value = true;

  await router.replace({
    name: "search",
    query: filtersToQuery(filters.value),
  });

  if (options.persistHistory) {
    historyEntries.value = addSearchHistoryEntry(filters.value);
  }

  try {
    results.value = await apiClient.search(payload, { signal: controller.signal });
    selectedHit.value = results.value.hits[0] ?? null;
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") {
      return;
    }

    results.value = null;
    selectedHit.value = null;
    searchError.value = resolveSearchErrorMessage(error, t);
  } finally {
    loading.value = false;
  }
}

function updateFilters(nextFilters: typeof filters.value) {
  filters.value = nextFilters;
}

async function rerunHistory(entry: SearchHistoryEntry) {
  filters.value = {
    query: entry.query,
    sourceKey: entry.sourceKey,
    publishedAfter: entry.publishedAfter,
    publishedBefore: entry.publishedBefore,
    limit: entry.limit,
  };
  await runSearch({ persistHistory: true });
}

function resetHistory() {
  clearSearchHistory();
  historyEntries.value = [];
}

function openHit(hit: SearchHit) {
  void router.push(buildSearchTarget(hit));
}

onMounted(async () => {
  filters.value = filtersFromQuery(route.query);
  loadHistory();
  await loadSources();

  if (filters.value.query) {
    await runSearch();
  }
});

onBeforeUnmount(() => {
  controller?.abort();
});
</script>

<template>
  <div class="search-page-layout">
    <section class="search-console-shell">
      <AppStateMessage
        v-if="sourceError"
        severity="error"
        :title="t('search.sourceErrorTitle')"
      >
        {{ sourceError }}
      </AppStateMessage>

      <SearchForm
        :filters="filters"
        :sources="sources"
        :busy="loading"
        @submit="runSearch({ persistHistory: true })"
        @update:filters="updateFilters"
      />

      <SearchHistoryPanel
        :entries="visibleHistoryEntries"
        @clear="resetHistory"
        @rerun="rerunHistory"
      />
    </section>

    <AppPanel
      v-if="showResultsPanel"
      class="search-results-panel"
      :title="t('search.resultsTitle')"
    >
      <AsyncStateBlock
        :loading="loading"
        :loading-title="t('search.scanningTitle')"
        :loading-message="t('search.scanningMessage')"
        loading-test-id="search-loading"
        :error="searchError"
        :empty="!searchError && searched && !!results && results.hits.length === 0"
      >
        <template #empty>
          <AppStateMessage :title="t('search.noMatchesTitle')">
            {{ t("search.noMatchesMessage") }}
          </AppStateMessage>
        </template>

        <div v-if="results" class="search-results-layout">
          <SearchResultList
            class="min-w-0"
            :hits="results.hits"
            :selected-hit="selectedHit"
            @open="openHit"
            @select="selectedHit = $event"
          />

          <SearchSelectionPreview
            class="hidden xl:block"
            :selected-hit="selectedHit"
            @open="openHit"
          />
        </div>
      </AsyncStateBlock>
    </AppPanel>
  </div>
</template>
