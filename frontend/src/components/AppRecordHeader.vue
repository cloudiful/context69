<script setup lang="ts">
defineProps<{
  title: string;
  subtitle?: string;
  description?: string;
}>();
</script>

<template>
  <div class="flex flex-wrap items-start justify-between gap-4">
    <div class="grid min-w-0 flex-1 gap-2">
      <div v-if="$slots.tags" class="flex flex-wrap items-center gap-2">
        <slot name="tags" />
      </div>
      <h3 class="text-lg font-semibold text-color">{{ title }}</h3>
      <p v-if="subtitle" class="text-sm text-muted-color">{{ subtitle }}</p>
      <p v-if="description" class="text-sm leading-6 text-muted-color">{{ description }}</p>
      <div v-if="$slots.meta" class="text-xs text-muted-color">
        <slot name="meta" />
      </div>
    </div>
    <div v-if="$slots.actions" class="flex shrink-0 items-center gap-2">
      <slot name="actions" />
    </div>
  </div>
</template>
