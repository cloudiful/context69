<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";

import SearchResultsPanel from "../components/SearchResultsPanel.vue";
import SearchToolbar from "../components/SearchToolbar.vue";
import { ApiError, type SearchHit, type SearchResponse } from "../services/api";
import { useSearchStream } from "../composables/use-search-stream";
import { useErrorToast } from "../composables/use-error-toast";
import type { SearchHistoryEntry } from "../utils/search-history";
import { replaySearchEntry } from "../utils/search-history";
import { resolveSearchErrorMessage } from "../utils/search-errors";
import { buildSearchTarget } from "../utils/search-target";
import {
  buildSearchPayload,
  createDefaultFilters,
  cursorFromQuery,
  filtersFromQuery,
  filtersToQuery,
  pageFromQuery,
  sameSearchFilters,
  saveSearchSession,
  type SearchNavigationState,
} from "../utils/search";
import type { SearchFilters } from "../types/ui";
import type { LocationQuery } from "vue-router";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const showErrorToast = useErrorToast();

const filters = ref<SearchFilters>(createDefaultFilters());
const results = ref<SearchResponse | null>(null);
const searched = ref(false);
const showResultsPanel = computed(() => loading.value || searched.value);

let isRouteSyncing = false;
let lastAppliedNav: SearchNavigationState = { page: 1 };

// The stream channel owns loading/refining state, SSE vs POST selection,
// cancellation and the subtle POST fallback notice.
const { loading, refining, fallbackNotice, run: runStream, abort: cancelStream } = useSearchStream({
  onResults: (response) => {
    results.value = response;
    searched.value = true;
    saveSearchSession(filters.value, response.pagination.page, lastAppliedNav.cursor);
  },
});

function navigationFromQuery(query: LocationQuery): SearchNavigationState {
  const cursor = cursorFromQuery(query);
  const page = pageFromQuery(query);
  return cursor ? { cursor } : { page };
}

function sameNavigation(left: SearchNavigationState, right: SearchNavigationState): boolean {
  return (left.cursor ?? null) === (right.cursor ?? null) && (left.page ?? 1) === (right.page ?? 1);
}

function isCursorOrderingError(error: unknown): boolean {
  return (
    error instanceof ApiError
    && error.status === 400
    && /cursor/i.test(error.message)
  );
}

async function runSearch(options: { cursor?: string | null; page?: number; syncRoute?: boolean } = {}) {
  const nav: SearchNavigationState = options.cursor
    ? { cursor: options.cursor }
    : { page: options.page ?? 1 };
  const payload = buildSearchPayload(filters.value, nav);

  if (!payload.query) {
    showErrorToast(null, t("search.emptyQuery"));
    results.value = null;
    searched.value = false;
    return;
  }

  lastAppliedNav = nav;
  const doSync = options.syncRoute !== false;
  if (doSync) {
    isRouteSyncing = true;
    try {
      await router.replace({
        name: "search",
        query: filtersToQuery(filters.value, nav),
      });
    } catch {
      // ignore navigation errors
    } finally {
      await nextTick();
      isRouteSyncing = false;
    }
  }

  try {
    await runStream(payload);
  } catch (error) {
    if (error instanceof Error && error.name === "AbortError") return;
    if (isCursorOrderingError(error)) {
      // The cursor belongs to a different ordering epoch (for example rerank
      // availability changed). Tell the user and restart from the first page
      // instead of silently showing a differently ordered window.
      showErrorToast(null, t("search.streaming.cursorError"));
      void runSearch({ page: 1 });
      return;
    }
    showErrorToast(null, resolveSearchErrorMessage(error, t));
  }
}

function handleToolbarSubmit() {
  void runSearch({ page: 1 });
}

function handleToolbarFilters(next: SearchFilters) {
  filters.value = next;
}

function handleToolbarGroupChange(groupPath: string | null) {
  filters.value = { ...filters.value, groupPath: groupPath ?? "" };
  void runSearch({ page: 1 });
}

function rerunHistory(entry: SearchHistoryEntry) {
  filters.value = replaySearchEntry(entry);
  void runSearch({ page: 1 });
}

function handlePrev() {
  const pagination = results.value?.pagination;
  if (!pagination) return;
  // Date mode is forward-only keyset pagination: there is no previous
  // window to step into. The Previous button is disabled in the UI but
  // the keyboard handler must not bypass that.
  if (filters.value.sort === "date") {
    return;
  }
  if (pagination.prev_cursor) {
    void runSearch({ cursor: pagination.prev_cursor });
  } else if (pagination.page > 1) {
    void runSearch({ page: pagination.page - 1 });
  }
}

function handleNext() {
  const pagination = results.value?.pagination;
  if (!pagination) return;
  if (pagination.next_cursor) {
    void runSearch({ cursor: pagination.next_cursor });
  } else if (pagination.has_more === true) {
    void runSearch({ page: pagination.page + 1 });
  }
}

function handlePageSize(size: number) {
  const normalized = Math.min(Math.max(size, 1), 50);
  filters.value = { ...filters.value, limit: normalized };
  void runSearch({ page: 1 });
}

function openHit(hit: SearchHit) {
  const page = results.value?.pagination.page ?? 1;
  saveSearchSession(filters.value, page, lastAppliedNav.cursor);
  void router.push(buildSearchTarget(hit));
}

watch(
  () => route.query,
  async (newQuery) => {
    if (isRouteSyncing) return;
    const nextFilters = filtersFromQuery(newQuery);
    const nextNav = navigationFromQuery(newQuery);
    const sameFilters = sameSearchFilters(filters.value, nextFilters);
    if (sameFilters && sameNavigation(lastAppliedNav, nextNav)) return;
    if (!nextFilters.query) {
      filters.value = nextFilters;
      results.value = null;
      searched.value = false;
      return;
    }
    filters.value = nextFilters;
    await runSearch({ ...nextNav, syncRoute: false });
  },
);

onMounted(async () => {
  const initialFilters = filtersFromQuery(route.query);
  const initialNav = navigationFromQuery(route.query);
  filters.value = initialFilters;
  if (initialFilters.query) {
    await runSearch({ ...initialNav, syncRoute: false });
  }
});

onBeforeUnmount(() => {
  cancelStream();
});
</script>

<template>
  <div class="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden">
    <SearchToolbar
      :filters="filters"
      :busy="loading"
      @update:filters="handleToolbarFilters"
      @group-change="handleToolbarGroupChange"
      @history-select="rerunHistory"
      @submit="handleToolbarSubmit"
    />

    <SearchResultsPanel
      v-if="showResultsPanel"
      class="min-h-0 flex-1"
      :results="results"
      :loading="loading"
      :refining="refining"
      :fallback-notice="fallbackNotice"
      :searched="searched"
      :highlight="filters.query"
      :sort="filters.sort"
      @open="openHit"
      @cancel="cancelStream"
      @prev="handlePrev"
      @next="handleNext"
      @page-size="handlePageSize"
    />
  </div>
</template>
