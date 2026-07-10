<script setup lang="ts">
import Card from "primevue/card";

defineProps<{
  label: string;
  value?: string | null;
  meta?: string | null;
}>();
</script>

<template>
  <Card>
    <template #content>
      <p class="mb-2 block text-xs font-medium uppercase tracking-[0.08em] text-app-text-dim">{{ label }}</p>
      <p v-if="value" class="text-sm text-app-text">{{ value }}</p>
      <p v-if="meta" class="text-xs text-app-text-dim">{{ meta }}</p>
      <slot />
    </template>
  </Card>
</template>
