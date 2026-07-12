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
    class="library-markdown-content max-h-[34rem] overflow-auto text-[0.98rem] leading-8 text-muted-color [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-4 [&_code]:rounded [&_code]:bg-emphasis [&_code]:px-1.5 [&_code]:py-0.5 [&_h1]:mt-6 [&_h1]:mb-3 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:text-color [&_h2]:mt-6 [&_h2]:mb-3 [&_h2]:text-xl [&_h2]:font-semibold [&_h2]:text-color [&_h3]:mt-6 [&_h3]:mb-3 [&_h3]:text-lg [&_h3]:font-semibold [&_h3]:text-color [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:my-3 [&_pre]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-surface [&_pre]:bg-surface-0 [&_pre]:px-4 [&_pre]:py-3 dark:[&_pre]:bg-surface-950 [&_ul]:my-3 [&_ul]:list-disc [&_ul]:pl-6"
    v-html="markdownHtml"
  />
  <pre v-else class="max-h-[34rem] overflow-auto whitespace-pre-wrap break-words font-sans text-[0.98rem] leading-8 text-muted-color">{{ content }}</pre>
</template>
