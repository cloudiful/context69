<script setup lang="ts">
withDefaults(defineProps<{
  showHeader?: boolean;
  title: string;
}>(), {
  showHeader: true,
});
</script>

<template>
  <div class="library-pane flex h-full min-h-[42rem] flex-col">
    <div v-if="showHeader" class="mb-3 flex flex-wrap items-center justify-between gap-2">
      <div>
        <h2 class="text-base font-semibold text-app-text">{{ title }}</h2>
        <p class="library-pane-description">{{ $t("library.previewLabel") }}</p>
      </div>
      <slot name="header-actions" />
    </div>

    <div class="min-h-0 flex-1 overflow-auto">
      <slot />
    </div>
  </div>
</template>
