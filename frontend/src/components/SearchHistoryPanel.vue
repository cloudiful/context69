<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

import AppPanel from "./AppPanel.vue";
import { searchHistoryClearButtonClass } from "../ui/button-classes";
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
        :class="searchHistoryClearButtonClass"
        @click="emit('clear')"
      >
        {{ t("search.history.clear") }}
      </Button>
    </template>

    <div class="flex flex-wrap gap-2">
      <button
        v-for="entry in entries"
        :key="`${entry.query}-${entry.sourceKey}-${entry.publishedAfter}-${entry.publishedBefore}-${entry.limit}`"
        class="inline-flex max-w-full items-center gap-2 rounded-full border border-app-border/70 bg-app-surface-muted/30 px-3 py-2 text-left transition hover:border-app-border-strong hover:bg-app-surface-soft/60"
        type="button"
        @click="emit('rerun', entry)"
      >
        <span class="truncate text-sm font-semibold text-app-text">{{ entry.query }}</span>
        <span v-if="formatHistorySummary(entry)" class="truncate text-xs text-app-text-dim">
          {{ formatHistorySummary(entry) }}
        </span>
      </button>
    </div>
  </AppPanel>
</template>
