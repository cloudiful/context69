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
      ? 'bg-(--p-content-hover-background)/55'
      : 'border border-dashed border-(--p-form-field-hover-border-color) bg-(--p-content-hover-background)'"
  >
    <p class="text-sm font-semibold text-(--p-text-color)">{{ title }}</p>
    <p class="mt-2 text-sm leading-6 text-(--p-text-muted-color)">{{ message }}</p>
  </div>
</template>
