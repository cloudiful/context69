<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { parseDate } from "@internationalized/date";
import type { DateValue } from "@internationalized/date";

import AppFormField from "./AppFormField.vue";
import type { SourceStatus } from "../services/api";
import type { SearchFilters } from "../types/ui";
import type { SearchHistoryEntry } from "../utils/search-history";

const props = defineProps<{
  filters: SearchFilters;
  historyEntries: SearchHistoryEntry[];
  sources: SourceStatus[];
  busy: boolean;
}>();

const emit = defineEmits<{
  "update:filters": [SearchFilters];
  "history-select": [SearchHistoryEntry];
  submit: [];
}>();

const { t } = useI18n();
const advancedFiltersOpen = ref(false);

function patchFilters(next: Partial<SearchFilters>) {
  emit("update:filters", {
    ...props.filters,
    ...next,
  });
}

function parseCalendarDate(value: string): DateValue | undefined {
  if (!value) return undefined;
  try {
    return parseDate(value);
  } catch {
    return undefined;
  }
}

function formatCalendarDate(value: DateValue | undefined): string {
  if (!value) return "";
  const year = String(value.year).padStart(4, "0");
  const month = String(value.month).padStart(2, "0");
  const day = String(value.day).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

type SearchDateRange = {
  start: DateValue | undefined;
  end: DateValue | undefined;
};

function updateQuery(value: string | SearchHistoryEntry | null) {
  if (value == null) {
    patchFilters({ query: "" });
    return;
  }
  if (typeof value === "object") {
    const entry = value as SearchHistoryEntry;
    const query = typeof entry.query === "string" ? entry.query : String(entry.query ?? "");
    if (!query || query === "[object Object]") {
      patchFilters({ query: "" });
      return;
    }
    patchFilters({ query });
    emit("history-select", { ...entry, query });
    return;
  }
  if (value === "[object Object]") {
    return;
  }
  const stringValue = String(value);
  patchFilters({ query: stringValue });
}

const sourceOptions = computed(() => [
  { label: t("search.form.allSources"), value: "__all__" },
  ...props.sources.map((source) => ({
    label: source.display_name === source.source_key
      ? source.display_name
      : `${source.display_name} (${source.source_key})`,
    value: source.source_key,
  })),
]);

const sourceModel = computed({
  get: () => props.filters.sourceKey || "__all__",
  set: (value: string) => patchFilters({ sourceKey: value === "__all__" ? "" : value }),
});

const limitModel = computed({
  get: () => props.filters.limit,
  set: (value: number | null) => {
    patchFilters({
      limit: typeof value === "number" && Number.isFinite(value) ? value : props.filters.limit,
    });
  },
});

const dateRangeModel = computed<SearchDateRange | null | undefined>({
  get: () => {
    const start = parseCalendarDate(props.filters.publishedAfter);
    const end = parseCalendarDate(props.filters.publishedBefore);
    if (!start && !end) return undefined;
    return { start, end };
  },
  set: (value) => {
    if (!value || (!value.start && !value.end)) {
      patchFilters({ publishedAfter: "", publishedBefore: "" });
      return;
    }
    patchFilters({
      publishedAfter: formatCalendarDate(value.start),
      publishedBefore: formatCalendarDate(value.end),
    });
  },
});

watch(
  () => [props.filters.publishedAfter, props.filters.publishedBefore],
  ([publishedAfter, publishedBefore]) => {
    if (publishedAfter || publishedBefore) {
      advancedFiltersOpen.value = true;
    }
  },
  { immediate: true },
);

function resetForm() {
  advancedFiltersOpen.value = false;
  emit("update:filters", {
    query: "",
    sourceKey: "",
    publishedAfter: "",
    publishedBefore: "",
    limit: 8,
  });
}
</script>

<template>
  <div class="block w-full min-w-0 overflow-hidden">
    <form class="grid w-full min-w-0 gap-2" @submit.prevent="emit('submit')">
      <div class="grid min-w-0 items-center gap-2 lg:grid-cols-[minmax(0,1fr)_auto]">
        <UInputMenu
          id="query"
          class="min-w-0"
          mode="autocomplete"
          :model-value="filters.query"
          :items="historyEntries"
          label-key="query"
          data-testid="search-query"
          size="sm"
          :placeholder="t('search.form.query')"
          :filter-fields="['query']"
          @update:model-value="updateQuery"
        >
          <template #item-label="{ item }">
            <span class="block truncate" :title="typeof item === 'string' ? item : (item as SearchHistoryEntry).query">{{ typeof item === 'string' ? item : (item as SearchHistoryEntry).query }}</span>
          </template>
        </UInputMenu>

        <div class="flex min-w-0 items-stretch justify-end gap-1.5">
          <UButton
            class="min-w-0 shrink-0"
            data-testid="search-toggle-advanced"
            type="button"
            variant="ghost"
            color="neutral"
            @click="advancedFiltersOpen = !advancedFiltersOpen"
          >
            {{ advancedFiltersOpen ? t("search.form.hideFilters") : t("search.form.moreFilters") }}
          </UButton>
          <UButton
            class="w-full min-w-0 lg:w-auto lg:min-w-[7.25rem] shrink-0"
            data-testid="search-submit"
            type="submit"
            :disabled="busy"
          >
            {{ busy ? t("search.form.running") : t("search.form.run") }}
          </UButton>
        </div>
      </div>

      <div v-if="advancedFiltersOpen" class="grid min-w-0 gap-2 overflow-hidden border-t border-default pt-2">
        <div class="grid min-w-0 gap-2 sm:grid-cols-2 lg:grid-cols-[minmax(0,1fr)_8rem]">
          <AppFormField input-id="source" :label="t('search.form.source')">
            <div class="min-w-0">
              <USelect
                id="source"
                v-model="sourceModel"
                data-testid="search-source"
                :items="sourceOptions"
                label-key="label"
                value-key="value"
                class="w-full min-w-0"
                :ui="{ content: 'max-w-[min(90vw,22rem)]' }"
              >
                <template #item-label="{ item }">
                  <span class="block truncate" :title="item.label">{{ item.label }}</span>
                </template>
              </USelect>
            </div>
          </AppFormField>

          <AppFormField input-id="limit" :label="t('search.form.limit')">
            <UInputNumber
              input-id="limit"
              v-model="limitModel"
              data-testid="search-limit"
              :min="1"
              :max="50"
              :use-grouping="false"
              class="w-full"
            />
          </AppFormField>
        </div>

        <div class="grid min-w-0 gap-2 sm:grid-cols-1">
          <AppFormField input-id="published-range" :label="t('search.form.publishedRange')">
            <UInputDate
              id="published-range"
              v-model="dateRangeModel"
              data-testid="search-published-range"
              range
              class="w-full min-w-0"
            />
          </AppFormField>
        </div>

        <div class="flex justify-end">
          <UButton
            class="min-w-20"
            data-testid="search-reset"
            type="button"
            color="neutral"
            variant="outline"
            @click="resetForm"
          >
            {{ t("common.reset") }}
          </UButton>
        </div>
      </div>
    </form>
  </div>
</template>
