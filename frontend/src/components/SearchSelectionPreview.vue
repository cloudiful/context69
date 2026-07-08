<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Button from "primevue/button";

import AppStateMessage from "./AppStateMessage.vue";
import type { SearchHit } from "../services/api";
import { formatDate, formatScore } from "../utils/format";

defineProps<{
  selectedHit: SearchHit | null;
}>();

const emit = defineEmits<{
  open: [SearchHit];
}>();

const { t } = useI18n();
</script>

<template>
  <aside class="min-w-0 rounded-[1.1rem] border border-app-border bg-app-surface-muted/36 p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] xl:sticky xl:top-3 xl:max-h-[calc(100vh-6rem)] xl:overflow-auto">
    <div v-if="selectedHit" class="grid gap-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p class="text-xs font-medium uppercase tracking-[0.18em] text-app-text-dim">{{ selectedHit.source_key }}</p>
          <h3 class="truncate text-base font-semibold text-app-text">{{ selectedHit.title }}</h3>
          <p class="text-xs leading-5 text-app-text-dim">
            {{ t("search.result.published", { date: formatDate(selectedHit.published_at) }) }}
            · {{ t("search.result.score") }} {{ formatScore(selectedHit.score) }}
          </p>
        </div>
        <Button
          class="shrink-0"
          type="button"
          severity="secondary"
          variant="outlined"
          size="small"
          @click="emit('open', selectedHit)"
        >
          {{ t("common.open") }}
        </Button>
      </div>

      <p v-if="selectedHit.library_path" class="text-xs leading-5 text-app-text-dim">
        {{ selectedHit.library_path }}<span v-if="selectedHit.library_section_label"> · {{ selectedHit.library_section_label }}</span>
      </p>

      <pre class="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-lg bg-app-surface-soft/72 px-3 py-2 text-sm leading-6 text-app-text-muted">{{ selectedHit.chunk_text }}</pre>
    </div>

    <AppStateMessage v-else :title="t('search.noMatchesTitle')">
      {{ t("search.noMatchesMessage") }}
    </AppStateMessage>
  </aside>
</template>
