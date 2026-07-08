<script setup lang="ts">
defineProps<{
  legend: string;
  description?: string;
}>();
</script>

<template>
  <section class="app-settings-section" :aria-label="legend">
    <div class="app-settings-section-body">
      <p v-if="description" class="app-settings-section-description">{{ description }}</p>
      <slot />
    </div>
  </section>
</template>
