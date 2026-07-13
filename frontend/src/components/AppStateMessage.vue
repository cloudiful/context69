<script setup lang="ts">
withDefaults(defineProps<{
  color?: "success" | "info" | "warning" | "error" | "neutral";
  title?: string;
}>(), {
  color: "neutral",
  title: "",
});
</script>

<template>
  <UAlert
    class="app-state-message"
    :color="color"
    variant="subtle"
    :title="title || undefined"
  >
    <template #description><slot /></template>
  </UAlert>
</template>
