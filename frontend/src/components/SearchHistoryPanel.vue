<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

import AppPanel from "./AppPanel.vue";
import type { SearchHistoryEntry } from "../utils/search-history";

defineProps<{
  entries: SearchHistoryEntry[];
}>();

const emit = defineEmits<{
  clear: [];
  rerun: [SearchHistoryEntry];
}>();

const { t } = useI18n();

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
</script>

<template>
  <AppPanel
    v-if="entries.length > 0"
    class="search-history-panel"
    :title="t('search.history.title')"
  >
    <template #actions>
      <Button
        class="search-history-clear"
        severity="secondary"
        variant="text"
        @click="emit('clear')"
      >
        {{ t("search.history.clear") }}
      </Button>
    </template>

    <div class="search-history-cloud">
      <button
        v-for="entry in entries"
        :key="`${entry.query}-${entry.sourceKey}-${entry.publishedAfter}-${entry.publishedBefore}-${entry.limit}`"
        class="search-history-chip"
        type="button"
        @click="emit('rerun', entry)"
      >
        <span class="search-history-chip-query">{{ entry.query }}</span>
        <span v-if="formatHistorySummary(entry)" class="search-history-chip-meta">
          {{ formatHistorySummary(entry) }}
        </span>
      </button>
    </div>
  </AppPanel>
</template>
