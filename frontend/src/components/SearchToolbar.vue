<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { apiClient, type GroupResponse, type SourceStatus } from "../services/api";
import type { SearchFilters } from "../types/ui";
import { useErrorToast } from "../composables/use-error-toast";
import { addSearchHistoryEntry, readSearchHistory, type SearchHistoryEntry } from "../utils/search-history";

import SearchForm from "./SearchForm.vue";

const props = withDefaults(defineProps<{
  filters: SearchFilters;
  busy?: boolean;
}>(), {
  busy: false,
});

const emit = defineEmits<{
  "update:filters": [SearchFilters];
  submit: [];
  "group-change": [string | null];
  "history-select": [SearchHistoryEntry];
}>();

const { t } = useI18n();
const showErrorToast = useErrorToast();
const sources = ref<SourceStatus[]>([]);
const groups = ref<GroupResponse[]>([]);
const historyEntries = ref<SearchHistoryEntry[]>([]);
const visibleHistoryEntries = computed(() => historyEntries.value.slice(0, 8));

async function loadSources() {
  try {
    const response = await apiClient.listSources({ page: 1, pageSize: 100, query: "" });
    sources.value = response.items;
  } catch (error) {
    showErrorToast(error, t("search.sourceLoadFailed"));
  }
}

async function loadGroups() {
  try {
    const response = await apiClient.listGroups({
      page: 1,
      page_size: 100,
      query: "",
    });
    groups.value = response.items;
  } catch (error) {
    // Folder scoping is optional; a failure only disables the filter.
    showErrorToast(error, t("search.form.groupLoadFailed"));
  }
}

function recordHistory(filters: SearchFilters) {
  historyEntries.value = addSearchHistoryEntry(filters);
}

function handleSubmit() {
  recordHistory(props.filters);
  emit("submit");
}

function handleHistorySelect(entry: SearchHistoryEntry) {
  emit("history-select", entry);
}

onMounted(async () => {
  historyEntries.value = readSearchHistory();
  await Promise.all([loadSources(), loadGroups()]);
});

defineExpose({ recordHistory });
</script>

<template>
  <section class="min-w-0 shrink-0 overflow-hidden py-1">
    <SearchForm
      :filters="filters"
      :sources="sources"
      :groups="groups"
      :busy="busy"
      :history-entries="visibleHistoryEntries"
      @group-change="emit('group-change', $event)"
      @history-select="handleHistorySelect"
      @submit="handleSubmit"
      @update:filters="emit('update:filters', $event)"
    />
  </section>
</template>
