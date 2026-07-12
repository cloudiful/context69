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
  <aside class="min-w-0 rounded-[1.1rem] border border-surface bg-emphasis p-3 xl:sticky xl:top-3 xl:max-h-[calc(100vh-6rem)] xl:overflow-auto">
    <div v-if="selectedHit" class="grid gap-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-color">{{ selectedHit.source_key }}</p>
          <h3 class="truncate text-base font-semibold text-color">{{ selectedHit.title }}</h3>
          <p class="text-xs leading-5 text-muted-color">
            {{ t("search.result.published", { date: formatDate(selectedHit.published_at) }) }}
            · {{ t("search.result.score") }} {{ formatScore(selectedHit.score) }}
          </p>
        </div>
        <Button
          class="shrink-0"
          type="button"
          size="small"
          severity="secondary"
          variant="outlined"
          @click="emit('open', selectedHit)"
        >
          {{ t("common.open") }}
        </Button>
      </div>

      <p v-if="selectedHit.library_path" class="text-xs leading-5 text-muted-color">
        {{ selectedHit.library_path }}<span v-if="selectedHit.library_section_label"> · {{ selectedHit.library_section_label }}</span>
      </p>

      <pre class="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-lg bg-emphasis px-3 py-2 text-sm leading-6 text-muted-color">{{ selectedHit.chunk_text }}</pre>
    </div>

    <AppStateMessage v-else :title="t('search.noMatchesTitle')">
      {{ t("search.noMatchesMessage") }}
    </AppStateMessage>
  </aside>
</template>
