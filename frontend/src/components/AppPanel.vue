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
  <section v-if="props.surface === 'plain'" class="app-panel app-panel-plain">
    <div v-if="props.title || $slots.actions || props.label" class="app-panel-header">
      <div class="min-w-0">
        <p v-if="props.label" class="section-label">{{ props.label }}</p>
        <h2 v-if="props.title" class="section-title">{{ props.title }}</h2>
      </div>
      <div class="app-panel-actions">
        <slot name="actions" />
      </div>
    </div>

    <div class="app-panel-content">
      <slot />
    </div>
  </section>

  <Card v-else class="app-panel">
    <template v-if="props.title || $slots.actions || props.label" #title>
      <div class="app-panel-header">
        <div class="min-w-0">
          <p v-if="props.label" class="section-label">{{ props.label }}</p>
          <h2 v-if="props.title" class="section-title">{{ props.title }}</h2>
        </div>
        <div class="app-panel-actions">
          <slot name="actions" />
        </div>
      </div>
    </template>

    <template #content>
      <div class="app-panel-content">
        <slot />
      </div>
    </template>
  </Card>
</template>
