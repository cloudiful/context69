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

function isHttpUri(value: string | null | undefined): boolean {
  if (!value) return false;
  return /^https?:\/\//i.test(value.trim());
}
</script>

<template>
  <UCard data-testid="search-selection-preview" class="min-w-0 overflow-hidden xl:sticky xl:top-3 xl:max-h-[calc(100vh-6rem)] xl:overflow-auto">
    <div v-if="selectedHit" class="grid min-w-0 gap-2">
      <div class="flex min-w-0 items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <h3 class="truncate text-base font-semibold text-color" :title="selectedHit.title">{{ selectedHit.title }}</h3>
          <p class="truncate text-xs leading-5 text-muted-color" :title="`${formatDate(selectedHit.published_at)} · ${formatScore(selectedHit.score)}`">
            {{ t("search.result.published", { date: formatDate(selectedHit.published_at) }) }}
            · {{ t("search.result.score") }} {{ formatScore(selectedHit.score) }}
          </p>
        </div>
        <UButton
          data-testid="search-preview-open"
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
        v-if="isHttpUri(selectedHit.source_uri)"
        :href="selectedHit.source_uri ?? undefined"
        target="_blank"
        rel="noopener noreferrer"
        class="block max-w-full truncate text-xs text-muted underline decoration-dotted underline-offset-2 hover:text-color"
        :title="selectedHit.source_uri ?? undefined"
      >{{ selectedHit.source_uri }}</a>

      <pre class="mt-1 max-h-[32rem] overflow-auto whitespace-pre-wrap break-words rounded-lg bg-emphasis px-3 py-2 text-sm leading-6 text-muted-color">{{ selectedHit.chunk_text }}</pre>
    </div>

    <UAlert
      v-else
      variant="subtle"
      :title="t('search.noMatchesTitle')"
      :description="t('search.noMatchesMessage')"
    />
  </UCard>
</template>
