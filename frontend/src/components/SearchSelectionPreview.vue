<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { SearchHit } from "../services/api";
import { formatDate, formatScore } from "../utils/format";
import { matchReasonTokens, stripTitlePrefix } from "../utils/search";
import MarkdownChunk from "./MarkdownChunk.vue";

const props = defineProps<{
  selectedHit: SearchHit | null;
  highlight?: string;
}>();

const emit = defineEmits<{
  open: [SearchHit];
}>();

const { t } = useI18n();

const displayContent = computed(() => {
  const hit = props.selectedHit;
  if (!hit) {
    return "";
  }
  // Chunks ingested from documents repeat the title on their first line
  // (标题：<title> ...); the preview header already shows it.
  return stripTitlePrefix(hit.chunk_text ?? "", hit.title ?? "");
});

const reasonTags = computed(() =>
  props.selectedHit ? matchReasonTokens(props.selectedHit.match_reason) : [],
);
const scoreTooltip = computed(() => {
  if (!props.selectedHit) {
    return "";
  }
  return t("search.result.scoreLabel", { score: formatScore(props.selectedHit.score) });
});

function reasonTagLabel(token: string): string {
  switch (token) {
    case "semantic":
      return t("search.result.reasonSemantic");
    case "title":
      return t("search.result.reasonTitle");
    case "keyword":
      return t("search.result.reasonKeyword");
    default:
      return token;
  }
}

function isHttpUri(value: string | null | undefined): boolean {
  if (!value) {
    return false;
  }
  return /^https?:\/\//i.test(value.trim());
}
</script>

<template>
  <div data-testid="search-selection-preview" class="grid min-w-0 gap-3">
    <div v-if="selectedHit" class="grid min-w-0 gap-2">
      <div class="flex min-w-0 items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <h3 class="truncate text-base font-semibold text-color" :title="selectedHit.title">{{ selectedHit.title }}</h3>
          <div class="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-5 text-muted-color">
            <span :title="scoreTooltip">{{ formatDate(selectedHit.published_at) }}</span>
            <span aria-hidden="true">·</span>
            <span class="min-w-0 truncate">{{ selectedHit.group_path }}</span>
          </div>
          <div v-if="reasonTags.length" class="mt-1 flex flex-wrap gap-1">
            <UBadge
              v-for="tag in reasonTags"
              :key="tag"
              :label="reasonTagLabel(tag)"
              color="primary"
              variant="subtle"
              size="sm"
            />
          </div>
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

      <div class="min-w-0">
        <MarkdownChunk :content="displayContent" markdown :highlight="highlight ?? ''" />
      </div>
    </div>

    <UAlert
      v-else
      variant="subtle"
      :title="t('search.noMatchesTitle')"
      :description="t('search.noMatchesMessage')"
    />
  </div>
</template>
