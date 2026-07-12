<script setup lang="ts">
withDefaults(defineProps<{
  id?: string;
  title?: string;
  description?: string;
  compact?: boolean;
}>(), {
  id: undefined,
  title: "",
  description: "",
  compact: false,
});
</script>

<template>
  <section
    :id="id"
    class="scroll-mt-16 grid border-t border-(--p-content-border-color)/60 first:border-t-0 first:pt-0"
    :class="compact ? 'gap-2.5 pt-3' : 'gap-4 pt-5'"
  >
    <div
      v-if="title || description || $slots.actions"
      class="flex flex-wrap items-start justify-between gap-3"
    >
      <div v-if="title || description" :class="compact ? 'grid gap-0.5' : 'grid gap-1'">
        <h3 v-if="title" class="text-sm font-semibold text-(--p-text-color)">{{ title }}</h3>
        <p v-if="description" class="text-sm leading-6 text-(--p-text-muted-color)">{{ description }}</p>
      </div>
      <slot name="actions" />
    </div>
    <slot />
  </section>
</template>
