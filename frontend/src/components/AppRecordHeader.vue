<script setup lang="ts">
defineProps<{
  title: string;
  subtitle?: string;
  description?: string;
}>();
</script>

<template>
  <div class="app-record-header">
    <div class="app-record-header-main">
      <div v-if="$slots.tags" class="app-record-header-tags">
        <slot name="tags" />
      </div>
      <h3 class="app-record-header-title">{{ title }}</h3>
      <p v-if="subtitle" class="app-record-header-subtitle">{{ subtitle }}</p>
      <p v-if="description" class="app-record-header-description">{{ description }}</p>
      <div v-if="$slots.meta" class="app-record-header-meta">
        <slot name="meta" />
      </div>
    </div>
    <div v-if="$slots.actions" class="app-record-header-actions">
      <slot name="actions" />
    </div>
  </div>
</template>
