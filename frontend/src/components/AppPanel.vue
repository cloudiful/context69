<script setup lang="ts">
import Card from "primevue/card";

const props = withDefaults(defineProps<{
  label?: string;
  surface?: "card" | "plain";
  title?: string;
}>(), {
  label: "",
  surface: "card",
  title: "",
});
</script>

<template>
  <section v-if="props.surface === 'plain'" class="grid min-w-0 gap-3">
    <div v-if="props.title || $slots.actions || props.label" class="flex items-start justify-between gap-3">
      <div class="min-w-0">
        <p v-if="props.label" class="text-xs font-medium uppercase tracking-[0.18em] text-(--p-text-muted-color)">{{ props.label }}</p>
        <h2 v-if="props.title" class="text-base font-semibold text-(--p-text-color)">{{ props.title }}</h2>
      </div>
      <div class="flex shrink-0 items-center gap-2">
        <slot name="actions" />
      </div>
    </div>

    <div class="grid min-w-0 gap-3">
      <slot />
    </div>
  </section>

  <Card v-else class="min-w-0">
    <template v-if="props.title || $slots.actions || props.label" #title>
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p v-if="props.label" class="text-xs font-medium uppercase tracking-[0.18em] text-(--p-text-muted-color)">{{ props.label }}</p>
          <h2 v-if="props.title" class="text-base font-semibold text-(--p-text-color)">{{ props.title }}</h2>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <slot name="actions" />
        </div>
      </div>
    </template>

    <template #content>
      <div class="grid min-w-0 gap-4">
        <slot />
      </div>
    </template>
  </Card>
</template>
