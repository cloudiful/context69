<script setup lang="ts">
import { computed, ref, watch } from "vue";

import type { LibraryPreviewContentFormat } from "../services/api";
import type { MarkdownPreviewBlock } from "../utils/markdown-preview-pagination";
import MarkdownChunk from "./MarkdownChunk.vue";

const props = defineProps<{
  content: string;
  contentFormat?: LibraryPreviewContentFormat | null;
}>();

const markdownPages = ref<MarkdownPreviewBlock[][]>([]);
const markdownPage = ref(1);

const normalizedContentFormat = computed<LibraryPreviewContentFormat>(() => {
  return props.contentFormat ?? "plain_text";
});
const isMarkdown = computed(() => normalizedContentFormat.value === "markdown");
const markdownPageCount = computed(() => markdownPages.value.length);
const markdownHtml = computed(() => markdownPages.value[markdownPage.value - 1]?.map((block) => block.html).join("") ?? "");

watch(markdownPageCount, (pageCount) => {
  markdownPage.value = pageCount === 0 ? 1 : Math.min(markdownPage.value, pageCount);
});

watch(
  [() => props.content, normalizedContentFormat],
  async ([content, contentFormat], _, onCleanup) => {
    let cancelled = false;
    onCleanup(() => {
      cancelled = true;
    });

    if (contentFormat !== "markdown") {
      markdownPages.value = [];
      markdownPage.value = 1;
      return;
    }

    try {
      const { renderMarkdownPreviewBlocks } = await import("../utils/markdown-preview");
      const { paginateMarkdownPreview } = await import("../utils/markdown-preview-pagination");
      if (cancelled) {
        return;
      }

      markdownPages.value = paginateMarkdownPreview(renderMarkdownPreviewBlocks(content));
      markdownPage.value = 1;
    } catch {
      if (cancelled) {
        return;
      }

      markdownPages.value = [];
    }
  },
  { immediate: true },
);
</script>

<template>
  <div :class="isMarkdown ? 'min-h-[20rem] md:min-h-[24rem]' : ''">
    <MarkdownChunk
      :content="content"
      :markdown="isMarkdown"
      :html="isMarkdown ? markdownHtml : null"
    />
  </div>
  <UPagination
    v-if="isMarkdown && markdownPageCount > 1"
    :page="markdownPage"
    :items-per-page="1"
    :total="markdownPageCount"
    size="sm"
    class="justify-center"
    @update:page="markdownPage = $event"
  />
</template>
