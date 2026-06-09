<script setup lang="ts">
defineProps<{
  path: string;
  title?: string;
}>();
</script>

<template>
  <svg
    viewBox="0 0 24 24"
    fill="currentColor"
    aria-hidden="true"
    focusable="false"
  >
    <title v-if="title">{{ title }}</title>
    <path :d="path" />
  </svg>
</template>
