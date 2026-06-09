<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import Button from "primevue/button";
import DatePicker from "primevue/datepicker";
import Fluid from "primevue/fluid";
import InputNumber from "primevue/inputnumber";
import InputText from "primevue/inputtext";
import Select from "primevue/select";

import AppFormField from "./AppFormField.vue";
import type { SourceStatus } from "../services/api";
import type { SearchFilters } from "../types/ui";

const props = defineProps<{
  filters: SearchFilters;
  sources: SourceStatus[];
  busy: boolean;
}>();

const emit = defineEmits<{
  "update:filters": [SearchFilters];
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

const queryModel = computed({
  get: () => props.filters.query,
  set: (value: string) => patchFilters({ query: value }),
});

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
  <Fluid>
    <form class="search-form-layout" @submit.prevent="emit('submit')">
      <div class="search-form-topline">
        <div class="search-form-query-shell">
          <InputText
            id="query"
            v-model="queryModel"
            data-testid="search-query"
            class="search-form-query"
            :placeholder="t('search.form.query')"
          />
        </div>

        <div class="search-form-actions">
          <Button
            class="search-form-action search-form-action-tertiary"
            data-testid="search-toggle-advanced"
            type="button"
            severity="secondary"
            variant="text"
            :label="advancedFiltersOpen ? t('search.form.hideFilters') : t('search.form.moreFilters')"
            @click="advancedFiltersOpen = !advancedFiltersOpen"
          />
          <Button
            class="search-form-action search-form-action-primary"
            data-testid="search-submit"
            type="submit"
            :disabled="busy"
            :label="busy ? t('search.form.running') : t('search.form.run')"
          />
        </div>
      </div>

      <div v-if="advancedFiltersOpen" class="search-form-advanced">
        <div class="search-form-grid search-form-grid-primary">
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

        <div class="search-form-grid search-form-grid-secondary">
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

        <div class="search-form-advanced-actions">
          <Button
            class="search-form-action search-form-action-secondary"
            data-testid="search-reset"
            type="button"
            severity="secondary"
            variant="outlined"
            :label="t('common.reset')"
            @click="resetForm"
          />
        </div>
      </div>
    </form>
  </Fluid>
</template>
