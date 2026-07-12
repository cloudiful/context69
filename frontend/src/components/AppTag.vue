<script setup lang="ts">
import { computed, useAttrs } from "vue";
import Tag from "primevue/tag";

defineOptions({ inheritAttrs: false });

const props = withDefaults(defineProps<{
  value?: string | number | null;
  severity?: "success" | "info" | "warn" | "danger" | "secondary" | "contrast";
}>(), {
  value: "",
  severity: "secondary",
});

const attrs = useAttrs();
const hasToolChipClass = computed(() => {
  const value = attrs.class;
  return typeof value === "string" ? value.split(/\s+/).includes("tool-chip") : false;
});

</script>

<template>
  <Tag
    v-bind="attrs"
    :class="hasToolChipClass ? 'max-w-48' : ''"
    :value="value"
    :severity="severity"
  />
</template>
