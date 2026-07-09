<script setup lang="ts">
defineProps<{
  legend: string;
  description?: string;
}>();
</script>

<template>
  <section class="app-settings-section" :aria-label="legend">
    <div class="grid gap-6">
      <p v-if="description" class="app-settings-section-description">{{ description }}</p>
      <slot />
    </div>
  </section>
</template>
