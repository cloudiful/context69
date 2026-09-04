<script setup lang="ts">
import { useI18n } from "vue-i18n";

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
  <UCard class="min-w-0 overflow-hidden xl:sticky xl:top-3 xl:max-h-[calc(100vh-6rem)] xl:overflow-auto">
    <div v-if="selectedHit" class="grid min-w-0 gap-3">
      <div class="flex min-w-0 items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <p class="truncate text-xs font-medium uppercase tracking-[0.18em] text-muted-color" :title="selectedHit.source_key">{{ selectedHit.source_key }}</p>
          <h3 class="truncate text-base font-semibold text-color" :title="selectedHit.title">{{ selectedHit.title }}</h3>
          <p class="truncate text-xs leading-5 text-muted-color" :title="`${formatDate(selectedHit.published_at)} · ${formatScore(selectedHit.score)}`">
            {{ t("search.result.published", { date: formatDate(selectedHit.published_at) }) }}
            · {{ t("search.result.score") }} {{ formatScore(selectedHit.score) }}
          </p>
          <p class="text-xs text-muted">
            <span class="truncate" :title="selectedHit.external_id">{{ selectedHit.external_id }}</span>
          </p>
        </div>
        <UButton
          class="shrink-0"
          type="button"
          size="sm"
          color="neutral"
          variant="outline"
          @click="emit('open', selectedHit)"
        >
          {{ t("common.open") }}
        </UButton>
      </div>

      <a
        v-if="selectedHit.source_uri"
        :href="selectedHit.source_uri"
        target="_blank"
        rel="noopener noreferrer"
        class="block max-w-full truncate text-xs text-muted underline decoration-dotted underline-offset-2 hover:text-color"
        :title="selectedHit.source_uri"
      >{{ selectedHit.source_uri }}</a>

      <p v-if="selectedHit.library_path" class="min-w-0 break-words text-xs leading-5 text-muted-color">
        {{ selectedHit.library_path }}<span v-if="selectedHit.library_section_label"> · {{ selectedHit.library_section_label }}</span>
      </p>

      <pre class="mt-2 max-h-[32rem] overflow-auto whitespace-pre-wrap break-words rounded-lg bg-emphasis px-3 py-2 text-sm leading-6 text-muted-color">{{ selectedHit.chunk_text }}</pre>
    </div>

    <UAlert
      v-else
      variant="subtle"
      :title="t('search.noMatchesTitle')"
      :description="t('search.noMatchesMessage')"
    />
  </UCard>
</template>
