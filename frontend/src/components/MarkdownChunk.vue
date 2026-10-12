<script setup lang="ts">
import { computed, ref, watch } from "vue";

import { highlightQueryText, highlightRenderedHtml } from "../utils/search-highlight";

const props = withDefaults(defineProps<{
  content: string;
  markdown?: boolean;
  highlight?: string;
  html?: string | null;
}>(), {
  markdown: false,
  highlight: "",
  html: null,
});

const markdownHtml = ref("");
const markdownReady = ref(false);

watch(
  () => [props.content, props.markdown, props.html],
  async () => {
    if (!props.markdown) {
      markdownHtml.value = "";
      markdownReady.value = false;
      return;
    }

    if (props.html !== null) {
      markdownHtml.value = props.html;
      markdownReady.value = true;
      return;
    }

    markdownReady.value = false;
    try {
      const { renderMarkdownPreviewBlocks } = await import("../utils/markdown-preview");
      markdownHtml.value = renderMarkdownPreviewBlocks(props.content)
        .map((block) => block.html)
        .join("");
      markdownReady.value = true;
    } catch {
      markdownHtml.value = "";
      markdownReady.value = false;
    }
  },
  { immediate: true },
);

// The markdown renderer already escaped its output, so marking only inserts
// `<mark>` around matched text runs and leaves tags and entities untouched. The
// plain fallback has no renderer, so it is escaped once by the shared wrapper.
const articleHtml = computed(() => highlightRenderedHtml(markdownHtml.value, props.highlight));
const preHtml = computed(() => highlightQueryText(props.content, props.highlight));
</script>

<template>
  <template v-if="markdown && markdownReady && articleHtml">
    <article
      class="library-markdown-content text-[0.98rem] leading-8 text-muted-color [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-4 [&_code]:rounded [&_code]:bg-emphasis [&_code]:px-1.5 [&_code]:py-0.5 [&_h1]:mt-6 [&_h1]:mb-3 [&_h1]:text-2xl [&_h1]:font-semibold [&_h1]:text-color [&_h2]:mt-6 [&_h2]:mb-3 [&_h2]:text-xl [&_h2]:font-semibold [&_h2]:text-color [&_h3]:mt-6 [&_h3]:mb-3 [&_h3]:text-lg [&_h3]:font-semibold [&_h3]:text-color [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:my-3 [&_pre]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:border [&_pre]:border-surface [&_pre]:bg-surface-0 [&_pre]:px-4 [&_pre]:py-3 dark:[&_pre]:bg-surface-950 [&_ul]:my-3 [&_ul]:list-disc [&_ul]:pl-6"
      v-html="articleHtml"
    />
  </template>
  <pre
    v-else
    class="whitespace-pre-wrap break-words font-sans text-[0.98rem] leading-8 text-muted-color"
    v-html="preHtml"
  />
</template>
