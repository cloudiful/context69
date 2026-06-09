<script setup lang="ts">
withDefaults(defineProps<{
  title: string;
  message: string;
  variant?: "default" | "soft";
}>(), {
  variant: "default",
});
</script>

<template>
  <div
    class="rounded-xl px-6 py-10 text-center"
    :class="variant === 'soft'
      ? 'bg-app-surface-soft/55'
      : 'border border-dashed border-app-border-strong bg-app-surface-muted'"
  >
    <p class="text-sm font-semibold text-app-text">{{ title }}</p>
    <p class="mt-2 text-sm leading-6 text-app-text-muted">{{ message }}</p>
  </div>
</template>
