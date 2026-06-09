<script setup lang="ts">
import Card from "primevue/card";

defineProps<{
  label: string;
  value?: string | null;
  meta?: string | null;
}>();
</script>

<template>
  <Card class="app-info-card">
    <template #content>
      <p class="form-label">{{ label }}</p>
      <p v-if="value" class="app-info-value">{{ value }}</p>
      <p v-if="meta" class="app-info-meta">{{ meta }}</p>
      <slot />
    </template>
  </Card>
</template>
