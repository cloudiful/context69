<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import AutoComplete from "primevue/autocomplete";
import Button from "primevue/button";
import DatePicker from "primevue/datepicker";
import Fluid from "primevue/fluid";
import InputNumber from "primevue/inputnumber";
import Select from "primevue/select";

import AppFormField from "./AppFormField.vue";
import type { SourceStatus } from "../services/api";
import type { SearchFilters } from "../types/ui";
import type { SearchHistoryEntry } from "../utils/search-history";
import {
  settingsPrimaryButtonClass,
  settingsSecondaryButtonClass,
  subtleTextButtonClass,
} from "../ui/button-classes";

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
  { label: t("search.form.allSources"), value: "" },
  ...props.sources.map((source) => ({
    label: source.display_name === source.source_key
      ? source.display_name
      : `${source.display_name} (${source.source_key})`,
    value: source.source_key,
  })),
]);

const sourceModel = computed({
  get: () => props.filters.sourceKey,
  set: (value: string) => patchFilters({ sourceKey: value }),
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
  get: () => parseDateValue(props.filters.publishedAfter),
  set: (value: Date | Date[] | null | undefined) => patchFilters({ publishedAfter: formatDateValue(value) }),
});

const publishedBeforeModel = computed({
  get: () => parseDateValue(props.filters.publishedBefore),
  set: (value: Date | Date[] | null | undefined) => patchFilters({ publishedBefore: formatDateValue(value) }),
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
  <Fluid class="block w-full">
    <form class="grid w-full gap-2 rounded-[1.1rem] border border-app-border/80 bg-app-surface/92 p-2" @submit.prevent="emit('submit')">
      <div class="grid items-center gap-2 lg:grid-cols-[minmax(0,1fr)_auto]">
        <div class="min-w-0">
          <AutoComplete
            input-id="query"
            :model-value="filters.query"
            :suggestions="historySuggestions"
            option-label="query"
            complete-on-focus
            :delay="0"
            data-testid="search-query"
            fluid
            input-class="min-h-[2.35rem] rounded-xl px-3 text-sm"
            :placeholder="t('search.form.query')"
            @complete="completeHistory"
            @update:model-value="updateQuery"
          />
        </div>

        <div class="flex items-stretch justify-end gap-1.5">
          <Button
            :class="[subtleTextButtonClass, 'min-w-0 justify-center']"
            data-testid="search-toggle-advanced"
            type="button"
            :label="advancedFiltersOpen ? t('search.form.hideFilters') : t('search.form.moreFilters')"
            @click="advancedFiltersOpen = !advancedFiltersOpen"
          />
          <Button
            :class="[settingsPrimaryButtonClass, 'min-h-[2.35rem] w-full min-w-0 justify-center whitespace-nowrap lg:w-auto lg:min-w-[7.25rem]']"
            data-testid="search-submit"
            type="submit"
            :disabled="busy"
            :label="busy ? t('search.form.running') : t('search.form.run')"
          />
        </div>
      </div>

      <div v-if="advancedFiltersOpen" class="grid gap-2 border-t border-app-border/70 pt-2">
        <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-[minmax(0,1fr)_8rem]">
          <AppFormField input-id="source" :label="t('search.form.source')">
            <Select
              input-id="source"
              v-model="sourceModel"
              data-testid="search-source"
              :options="sourceOptions"
              option-label="label"
              option-value="value"
            />
          </AppFormField>

          <AppFormField input-id="limit" :label="t('search.form.limit')">
            <InputNumber
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
            <DatePicker
              input-id="published-after"
              v-model="publishedAfterModel"
              data-testid="search-published-after"
              date-format="yy-mm-dd"
              show-icon
            />
          </AppFormField>

          <AppFormField input-id="published-before" :label="t('search.form.publishedBefore')">
            <DatePicker
              input-id="published-before"
              v-model="publishedBeforeModel"
              data-testid="search-published-before"
              date-format="yy-mm-dd"
              show-icon
            />
          </AppFormField>
        </div>

        <div class="flex justify-end">
          <Button
            :class="[settingsSecondaryButtonClass, 'min-w-20 justify-center']"
            data-testid="search-reset"
            type="button"
            :label="t('common.reset')"
            @click="resetForm"
          />
        </div>
      </div>
    </form>
  </Fluid>
</template>
