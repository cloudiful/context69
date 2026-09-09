<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { SearchHit, SearchPagination } from "../services/api";
import { formatDate, formatScore } from "../utils/format";
import { buildSnippet, highlightQueryText, matchReasonTokens, stripTitlePrefix } from "../utils/search";

const props = withDefaults(defineProps<{
  hits: SearchHit[];
  pagination: SearchPagination;
  selectedHit?: SearchHit | null;
  highlight?: string;
  /**
   * The active sort mode for the request that produced `pagination`.
   * Date mode is forward-only keyset pagination, so the previous button
   * is explicitly disabled.
   */
  sort?: "relevance" | "date";
}>(), {
  selectedHit: null,
  highlight: "",
  sort: "relevance",
});

const emit = defineEmits<{
  preview: [SearchHit];
  open: [SearchHit];
  prev: [];
  next: [];
  "page-size": [number];
}>();

const { t } = useI18n();

const pageSizeOptions = computed(() => {
  const base = [8, 16, 24, 50];
  const current = props.pagination.page_size;
  if (base.includes(current)) {
    return base;
  }
  return [...base, current].sort((a, b) => a - b);
});
const pageSizeItems = computed(() =>
  pageSizeOptions.value.map((value) => ({ label: String(value), value })),
);

// Cursor-first navigation: prev/next use the ordering-epoch cursors issued by
// the page. Legacy responses without cursors (short-lived cached pages) fall
// back to plain page stepping so users never get stuck. Date mode is
// forward-only keyset pagination, so the previous button is explicitly
// disabled (no cursor would take the user back) instead of presenting a
// broken `page - 1` step.
const canGoPrev = computed(() => {
  if (props.sort === "date") {
    return false;
  }
  return props.pagination.prev_cursor != null || props.pagination.page > 1;
});
const canGoNext = computed(
  () => props.pagination.next_cursor != null || props.pagination.has_more === true,
);

function updatePageSize(value: unknown) {
  const next = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(next) || next === props.pagination.page_size) return;
  emit("page-size", next);
}

function isSelected(hit: SearchHit): boolean {
  return !!props.selectedHit && props.selectedHit.chunk_id === hit.chunk_id;
}

function isHttpUri(value: string | null | undefined): boolean {
  if (!value) {
    return false;
  }
  return /^https?:\/\//i.test(value.trim());
}

function snippetHtml(hit: SearchHit): string {
  const body = stripTitlePrefix(hit.chunk_text ?? "", hit.title ?? "");
  return highlightQueryText(buildSnippet(body, props.highlight), props.highlight);
}

function scoreTooltip(hit: SearchHit): string {
  return t("search.result.scoreLabel", { score: formatScore(hit.score) });
}

function titleTooltip(hit: SearchHit): string {
  const parts = [scoreTooltip(hit)];
  if (hit.published_at) {
    parts.push(formatDate(hit.published_at));
  }
  if (hit.group_path) {
    parts.push(hit.group_path);
  }
  return parts.join(" · ");
}

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
</script>

<template>
  <div data-testid="search-results-list" class="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden">
    <div data-testid="search-results-scroll" class="h-full min-h-[220px] min-w-0 overflow-y-auto overscroll-contain">
      <ul class="grid min-w-0 gap-px">
        <li
          v-for="hit in hits"
          :key="hit.chunk_id"
          data-testid="search-result-item"
          class="min-w-0 rounded-md px-2 py-2"
          :class="isSelected(hit) ? 'bg-muted/40 ring-1 ring-default' : ''"
          :data-selected="isSelected(hit) ? '' : undefined"
        >
          <div class="flex min-w-0 items-start justify-between gap-2">
            <UButton
              data-testid="search-result-select"
              class="min-w-0 flex-1 justify-start px-0 text-left text-sm font-semibold leading-6 [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:2]"
              type="button"
              variant="link"
              size="sm"
              color="neutral"
              :aria-selected="isSelected(hit)"
              :title="titleTooltip(hit)"
              @click="emit('preview', hit)"
            >
              {{ hit.title }}
            </UButton>
            <div class="flex shrink-0 items-center gap-1">
              <UButton
                data-testid="search-result-preview"
                type="button"
                variant="ghost"
                size="sm"
                color="neutral"
                class="shrink-0"
                @click="emit('preview', hit)"
              >
                {{ t("search.result.preview") }}
              </UButton>
              <UButton
                data-testid="search-result-open"
                type="button"
                variant="ghost"
                size="sm"
                color="neutral"
                class="shrink-0"
                @click="emit('open', hit)"
              >
                {{ t("common.open") }}
              </UButton>
              <UButton
                v-if="isHttpUri(hit.source_uri)"
                data-testid="search-result-source-link"
                variant="ghost"
                size="sm"
                color="neutral"
                icon="i-lucide-external-link"
                square
                :href="hit.source_uri ?? undefined"
                target="_blank"
                rel="noopener"
                :title="hit.source_uri ?? undefined"
                :aria-label="t('search.result.openSourceLink')"
              />
            </div>
          </div>

          <p v-if="!isHttpUri(hit.source_uri) && hit.is_library_file && hit.library_path" class="mt-1 max-w-full truncate text-xs text-muted">
            {{ hit.library_path }}
          </p>

          <p
            class="mt-1 min-w-0 break-words text-[0.88rem] leading-[1.5rem] text-muted-color [display:-webkit-box] [overflow:hidden] [-webkit-box-orient:vertical] [-webkit-line-clamp:3]"
            v-html="snippetHtml(hit)"
          />

          <div class="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-5 text-muted">
            <span>{{ formatDate(hit.published_at) }}</span>
            <span aria-hidden="true">·</span>
            <span class="min-w-0 truncate">{{ hit.group_path }}</span>
            <span v-for="tag in matchReasonTokens(hit.match_reason)" :key="tag">
              <UBadge :label="reasonTagLabel(tag)" color="primary" variant="subtle" size="sm" />
            </span>
          </div>
        </li>
      </ul>

      <div v-if="hits.length === 0" class="grid min-h-[220px] place-items-center p-4">
        <p class="text-sm text-muted">{{ t("search.noMatchesMessage") }}</p>
      </div>
    </div>

    <div
      v-if="props.pagination && props.pagination.total >= 0"
      class="flex min-h-9 shrink-0 flex-wrap items-center justify-between gap-2 overflow-hidden border-t border-default"
    >
      <div v-if="props.pagination.total > 0" class="flex items-center gap-2">
        <span class="text-sm text-muted-color">{{ t("common.itemsPerPage") }}</span>
        <USelect
          :model-value="props.pagination.page_size"
          :items="pageSizeItems"
          value-key="value"
          class="w-20 min-w-0"
          :aria-label="t('common.itemsPerPage')"
          data-testid="search-page-size"
          @update:model-value="updatePageSize($event)"
        />
      </div>
      <div v-else class="flex items-center">
        <span class="text-sm text-muted">{{ t("common.itemsPerPage") }}</span>
      </div>

      <div class="flex min-w-0 items-center gap-1.5">
        <UButton
          data-testid="search-prev"
          type="button"
          variant="outline"
          size="sm"
          color="neutral"
          icon="i-lucide-chevron-left"
          :disabled="!canGoPrev"
          :title="props.sort === 'date' ? t('search.pagination.previousDisabledHint') : t('search.pagination.previous')"
          :aria-label="t('search.pagination.previous')"
          @click="emit('prev')"
        >
          {{ t("search.pagination.previous") }}
        </UButton>
        <span class="min-w-0 px-1 text-xs tabular-nums text-muted">Page {{ props.pagination.page }}</span>
        <UButton
          data-testid="search-next"
          type="button"
          variant="outline"
          size="sm"
          color="neutral"
          icon="i-lucide-chevron-right"
          :disabled="!canGoNext"
          :aria-label="t('search.pagination.next')"
          @click="emit('next')"
        >
          {{ t("search.pagination.next") }}
        </UButton>
      </div>
    </div>
  </div>
</template>
