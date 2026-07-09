<script setup lang="ts">
withDefaults(defineProps<{
  id?: string;
  title?: string;
  description?: string;
}>(), {
  id: undefined,
  title: "",
  description: "",
});
</script>

<template>
  <section
    :id="id"
    class="scroll-mt-16 grid gap-4 border-t border-app-border/60 pt-5 first:border-t-0 first:pt-0"
  >
    <div
      v-if="title || description || $slots.actions"
      class="flex flex-wrap items-start justify-between gap-3"
    >
      <div v-if="title || description" class="grid gap-1">
        <h3 v-if="title" class="text-sm font-semibold text-app-text">{{ title }}</h3>
        <p v-if="description" class="text-sm leading-6 text-app-text-dim">{{ description }}</p>
      </div>
      <slot name="actions" />
    </div>
    <slot />
  </section>
</template>
