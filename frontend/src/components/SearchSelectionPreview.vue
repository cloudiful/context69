<script setup lang="ts">
import { useI18n } from "vue-i18n";

import AppStateMessage from "./AppStateMessage.vue";
import type { SearchHit } from "../services/api";
import { controlButtonClass } from "../ui/button-classes";
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
  <aside class="search-selection-panel">
    <div v-if="selectedHit" class="grid gap-3">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p class="section-label">{{ selectedHit.source_key }}</p>
          <h3 class="truncate text-base font-semibold text-app-text">{{ selectedHit.title }}</h3>
          <p class="text-xs leading-5 text-app-text-dim">
            {{ t("search.result.published", { date: formatDate(selectedHit.published_at) }) }}
            · {{ t("search.result.score") }} {{ formatScore(selectedHit.score) }}
          </p>
        </div>
        <button
          :class="[controlButtonClass, 'shrink-0']"
          type="button"
          @click="emit('open', selectedHit)"
        >
          {{ t("common.open") }}
        </button>
      </div>

      <p v-if="selectedHit.library_path" class="text-xs leading-5 text-app-text-dim">
        {{ selectedHit.library_path }}<span v-if="selectedHit.library_section_label"> · {{ selectedHit.library_section_label }}</span>
      </p>

      <pre class="content-pre search-selection-content">{{ selectedHit.chunk_text }}</pre>
    </div>

    <AppStateMessage v-else :title="t('search.noMatchesTitle')">
      {{ t("search.noMatchesMessage") }}
    </AppStateMessage>
  </aside>
</template>
