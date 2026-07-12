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
      ? 'bg-emphasis'
      : 'border border-dashed border-surface bg-emphasis'"
  >
    <p class="text-sm font-semibold text-color">{{ title }}</p>
    <p class="mt-2 text-sm leading-6 text-muted-color">{{ message }}</p>
  </div>
</template>
