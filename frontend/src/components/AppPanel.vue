<script setup lang="ts">
import Card from "primevue/card";

defineProps<{
  label?: string;
  title: string;
}>();
</script>

<template>
  <Card class="app-panel">
    <template #title>
      <div class="app-panel-header">
        <div class="min-w-0">
          <p v-if="label" class="section-label">{{ label }}</p>
          <h2 class="section-title">{{ title }}</h2>
        </div>
        <div class="app-panel-actions">
          <slot name="actions" />
        </div>
      </div>
    </template>

    <template #content>
      <div class="app-panel-content">
        <slot />
      </div>
    </template>
  </Card>
</template>
