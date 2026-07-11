<script setup lang="ts">
import { computed, ref, watch } from "vue";

import type { LibraryPreviewContentFormat } from "../services/api";

const props = defineProps<{
  content: string;
  contentFormat?: LibraryPreviewContentFormat | null;
}>();

const markdownHtml = ref("");
const markdownReady = ref(false);

const normalizedContentFormat = computed<LibraryPreviewContentFormat>(() => {
  return props.contentFormat ?? "plain_text";
});

watch(
  [() => props.content, normalizedContentFormat],
  async ([content, contentFormat], _, onCleanup) => {
    let cancelled = false;
    onCleanup(() => {
      cancelled = true;
    });

    if (contentFormat !== "markdown") {
      markdownHtml.value = "";
      markdownReady.value = false;
      return;
    }

    markdownReady.value = false;

    try {
      const { renderMarkdownPreview } = await import("../utils/markdown-preview");
      if (cancelled) {
        return;
      }

      markdownHtml.value = renderMarkdownPreview(content);
    } catch {
      if (cancelled) {
        return;
      }

      markdownHtml.value = "";
    } finally {
      if (!cancelled) {
        markdownReady.value = true;
      }
    }
  },
  { immediate: true },
);
</script>

<template>
  <article
    v-if="normalizedContentFormat === 'markdown' && markdownReady && markdownHtml"
    class="library-markdown-content max-h-[34rem] overflow-auto text-[0.98rem] leading-8 text-app-text-muted"
    v-html="markdownHtml"
  />
  <pre v-else class="max-h-[34rem] overflow-auto whitespace-pre-wrap break-words font-sans text-[0.98rem] leading-8 text-app-text-muted">{{ content }}</pre>
</template>
