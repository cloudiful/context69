<script setup lang="ts">
import { computed, ref, watch } from "vue";

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

const escapedHighlight = computed(() => escapeHtml(props.highlight.trim()));

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function highlightText(text: string, term: string): string {
  if (!term) {
    return text;
  }
  const parts = text.split(term);
  if (parts.length === 1) {
    return text;
  }
  return parts.join(`<mark>${term}</mark>`);
}

// Wrap escaped highlight occurrences in <mark>, leaving real tags untouched.
function highlightHtml(html: string, term: string): string {
  if (!term) {
    return html;
  }
  let output = "";
  let index = 0;
  while (index < html.length) {
    const tagStart = html.indexOf("<", index);
    if (tagStart === -1) {
      output += highlightText(html.slice(index), term);
      break;
    }
    output += highlightText(html.slice(index, tagStart), term);
    const tagEnd = html.indexOf(">", tagStart);
    if (tagEnd === -1) {
      output += html.slice(tagStart);
      break;
    }
    output += html.slice(tagStart, tagEnd + 1);
    index = tagEnd + 1;
  }
  return output;
}

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

const articleHtml = computed(() => highlightHtml(markdownHtml.value, escapedHighlight.value));
const preHtml = computed(() => highlightHtml(escapeHtml(props.content), escapedHighlight.value));
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
