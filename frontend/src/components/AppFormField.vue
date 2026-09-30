<script setup lang="ts">
import { computed } from "vue";

import UFormField from "@nuxt/ui/components/FormField.vue";

const props = withDefaults(defineProps<{
  inputId: string;
  label: string;
  helper?: string;
  layout?: "stacked" | "inline";
  /** Links the field to UForm validation errors; defaults to inputId. */
  name?: string;
}>(), {
  helper: "",
  layout: "stacked",
  name: "",
});

const fieldName = computed(() => props.name || props.inputId);
const orientation = computed(() => (props.layout === "inline" ? "horizontal" : "vertical"));

// The label keeps the compact uppercase settings style; the container spacing
// mirrors the previous handwritten grid so the surrounding sections do not shift.
const fieldUi = computed(() => ({
  root: props.layout === "inline" ? "min-w-0" : "grid min-w-0 content-start self-start gap-2",
  label: "text-xs font-medium uppercase tracking-[0.08em] text-muted",
  container: "grid min-w-0 gap-2",
}));
</script>

<template>
  <UFormField
    :name="fieldName"
    :label="props.label"
    :help="props.helper || undefined"
    :orientation="orientation"
    :ui="fieldUi"
  >
    <slot />
  </UFormField>
</template>
