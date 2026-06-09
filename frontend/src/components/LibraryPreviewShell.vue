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
    <div v-if="showHeader" class="library-pane-header">
      <div>
        <h2 class="section-title">{{ title }}</h2>
        <p class="library-pane-description">{{ $t("library.previewLabel") }}</p>
      </div>
      <slot name="header-actions" />
    </div>

    <div class="split-panel-body flex-1 overflow-auto">
      <slot />
    </div>
  </div>
</template>
