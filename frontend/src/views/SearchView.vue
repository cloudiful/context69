<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

import AsyncStateBlock from "../components/AsyncStateBlock.vue";
import AppPanel from "../components/AppPanel.vue";
import AppStateMessage from "../components/AppStateMessage.vue";
import SearchForm from "../components/SearchForm.vue";
import SearchResultList from "../components/SearchResultList.vue";
import { apiClient, type SearchHit, type SearchResponse, type SourceStatus } from "../services/api";
import { formatDate, formatScore } from "../utils/format";
import { addSearchHistoryEntry, clearSearchHistory, readSearchHistory, type SearchHistoryEntry } from "../utils/search-history";
import { createDefaultFilters, filtersFromQuery, filtersToQuery, buildSearchPayload } from "../utils/search";

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
    searchError.value = error instanceof Error ? error.message : t("search.searchFailed");
  } finally {
    loading.value = false;
  }
}

function updateFilters(nextFilters: typeof filters.value) {
  filters.value = nextFilters;
}

function formatHistorySummary(entry: SearchHistoryEntry) {
  const parts = [];

  if (entry.sourceKey) {
    parts.push(t("search.history.sourceValue", { source: entry.sourceKey }));
  }
  if (entry.publishedAfter || entry.publishedBefore) {
    parts.push(t("search.history.dateRange", {
      after: entry.publishedAfter || "—",
      before: entry.publishedBefore || "—",
    }));
  }

  return parts.join(" · ");
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
  void router.push(buildTarget(hit));
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

      <AppPanel
        v-if="visibleHistoryEntries.length > 0"
        class="search-history-panel"
        :title="t('search.history.title')"
      >
        <template #actions>
          <Button
            class="search-history-clear"
            severity="secondary"
            variant="text"
            @click="resetHistory"
          >
            {{ t("search.history.clear") }}
          </Button>
        </template>

        <div class="search-history-cloud">
          <button
            v-for="entry in visibleHistoryEntries"
            :key="`${entry.query}-${entry.sourceKey}-${entry.publishedAfter}-${entry.publishedBefore}-${entry.limit}`"
            class="search-history-chip"
            type="button"
            @click="rerunHistory(entry)"
          >
            <span class="search-history-chip-query">{{ entry.query }}</span>
            <span v-if="formatHistorySummary(entry)" class="search-history-chip-meta">
              {{ formatHistorySummary(entry) }}
            </span>
          </button>
        </div>
      </AppPanel>
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

        <div v-if="results" class="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(24rem,32rem)] xl:items-start">
          <SearchResultList
            class="min-w-0"
            :hits="results.hits"
            :selected-hit="selectedHit"
            @open="openHit"
            @select="selectedHit = $event"
          />

          <aside class="hidden min-w-0 rounded-[1.1rem] border border-app-border bg-app-surface-muted/36 p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] xl:sticky xl:top-3 xl:block xl:max-h-[calc(100vh-6rem)] xl:overflow-auto">
            <div v-if="selectedHit" class="grid gap-3">
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                  <p class="section-label">{{ selectedHit.source_key }}</p>
                  <h3 class="truncate text-base font-semibold text-app-text">{{ selectedHit.title }}</h3>
                  <p class="text-xs leading-5 text-app-text-dim">
                    {{ t("search.result.published", { date: formatDate(selectedHit.published_at) }) }}
                    · {{ t("search.result.score") }} {{ formatScore(selectedHit.score) }}
                  </p>
                </div>
                <button class="min-h-8 shrink-0 rounded-lg border border-app-border bg-app-surface-soft/45 px-3 text-sm font-medium text-app-text shadow-none transition hover:border-app-border-strong hover:bg-app-surface-soft/72" type="button" @click="openHit(selectedHit)">
                  {{ t("common.open") }}
                </button>
              </div>

              <p v-if="selectedHit.library_path" class="text-xs leading-5 text-app-text-dim">
                {{ selectedHit.library_path }}<span v-if="selectedHit.library_section_label"> · {{ selectedHit.library_section_label }}</span>
              </p>

              <pre class="content-pre max-h-[calc(100vh-17rem)]">{{ selectedHit.chunk_text }}</pre>
            </div>

            <AppStateMessage v-else :title="t('search.noMatchesTitle')">
              {{ t("search.noMatchesMessage") }}
            </AppStateMessage>
          </aside>
        </div>
      </AsyncStateBlock>
    </AppPanel>
  </div>
</template>
