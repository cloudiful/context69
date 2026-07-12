<script setup lang="ts">
defineProps<{
  legend: string;
  description?: string;
}>();
</script>

<template>
  <section class="min-w-0" :aria-label="legend">
    <div class="grid gap-6">
      <p v-if="description" class="text-sm leading-6 text-(--p-text-muted-color)">{{ description }}</p>
      <slot />
    </div>
  </section>
</template>
