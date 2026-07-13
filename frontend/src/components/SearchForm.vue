<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

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
const historySuggestions = ref<SearchHistoryEntry[]>([]);

function patchFilters(next: Partial<SearchFilters>) {
  emit("update:filters", {
    ...props.filters,
    ...next,
  });
}

function parseDateValue(value: string): Date | null {
  if (!value) {
    return null;
  }

  const [year, month, day] = value.split("-").map((item) => Number.parseInt(item, 10));
  if (!year || !month || !day) {
    return null;
  }

  return new Date(year, month - 1, day);
}

function formatDateValue(value: Date | Date[] | null | undefined): string {
  const date = Array.isArray(value) ? value[0] : value;
  if (!(date instanceof Date) || Number.isNaN(date.getTime())) {
    return "";
  }

  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function completeHistory(event: { query: string }) {
  const query = event.query.trim().toLocaleLowerCase();
  historySuggestions.value = props.historyEntries.filter((entry) => !query || entry.query.toLocaleLowerCase().includes(query));
}

function updateQuery(value: string | SearchHistoryEntry | null) {
  if (value && typeof value === "object") {
    emit("history-select", value);
    return;
  }
  patchFilters({ query: value ?? "" });
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

const publishedAfterModel = computed({
  get: () => props.filters.publishedAfter,
  set: (value: string) => patchFilters({ publishedAfter: value }),
});

const publishedBeforeModel = computed({
  get: () => props.filters.publishedBefore,
  set: (value: string) => patchFilters({ publishedBefore: value }),
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
  <div class="block w-full">
    <form class="grid w-full gap-2" @submit.prevent="emit('submit')">
      <div class="grid items-center gap-2 lg:grid-cols-[minmax(0,1fr)_auto]">
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
          @update:model-value="updateQuery"
        />

        <div class="flex items-stretch justify-end gap-1.5">
          <UButton
            class="min-w-0"
            data-testid="search-toggle-advanced"
            type="button"
      variant="ghost"            color="neutral"
            @click="advancedFiltersOpen = !advancedFiltersOpen"
          >
            {{ advancedFiltersOpen ? t("search.form.hideFilters") : t("search.form.moreFilters") }}
          </UButton>
          <UButton
            class="w-full min-w-0 lg:w-auto lg:min-w-[7.25rem]"
            data-testid="search-submit"
            type="submit"
            :disabled="busy"
          >
            {{ busy ? t("search.form.running") : t("search.form.run") }}
          </UButton>
        </div>
      </div>

      <div v-if="advancedFiltersOpen" class="grid gap-2 border-t border-surface pt-2">
        <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-[minmax(0,1fr)_8rem]">
          <AppFormField input-id="source" :label="t('search.form.source')">
            <USelect
              id="source"
              v-model="sourceModel"
              data-testid="search-source"
              :items="sourceOptions"
              label-key="label"
              value-key="value"
            />
          </AppFormField>

          <AppFormField input-id="limit" :label="t('search.form.limit')">
            <UInputNumber
              input-id="limit"
              v-model="limitModel"
              data-testid="search-limit"
              :min="1"
              :max="50"
              :use-grouping="false"
            />
          </AppFormField>
        </div>

        <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-[minmax(0,12rem)_minmax(0,12rem)]">
          <AppFormField input-id="published-after" :label="t('search.form.publishedAfter')">
            <UInput
              id="published-after"
              v-model="publishedAfterModel"
              data-testid="search-published-after"
              type="date"
            />
          </AppFormField>

          <AppFormField input-id="published-before" :label="t('search.form.publishedBefore')">
            <UInput
              id="published-before"
              v-model="publishedBeforeModel"
              data-testid="search-published-before"
              type="date"
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
