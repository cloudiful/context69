<script setup lang="ts">
import { computed, useAttrs } from "vue";

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

const severityClass = computed(() => {
  switch (props.severity) {
    case "success":
      return "border-[color-mix(in_srgb,var(--color-app-success)_34%,transparent)] bg-[color-mix(in_srgb,var(--color-app-success)_18%,transparent)] text-[color-mix(in_srgb,var(--color-app-success)_78%,white_22%)]";
    case "danger":
      return "bg-[color-mix(in_srgb,var(--color-app-danger)_18%,transparent)] text-[color-mix(in_srgb,var(--color-app-danger)_78%,white_22%)]";
    case "warn":
      return "bg-[color-mix(in_srgb,var(--color-app-warning)_18%,transparent)] text-[color-mix(in_srgb,var(--color-app-warning)_78%,white_22%)]";
    default:
      return "";
  }
});
</script>

<template>
  <span
    v-bind="attrs"
    :class="[
      'inline-flex min-h-6 items-center rounded-[0.45rem] border border-[color-mix(in_srgb,var(--color-app-border)_82%,transparent)] bg-[color-mix(in_srgb,var(--color-app-surface-soft)_48%,var(--color-app-surface)_52%)] px-2 py-[0.18rem] text-[0.78rem] font-semibold leading-[1.15rem] text-app-text-muted',
      severityClass,
      hasToolChipClass ? 'max-w-48' : '',
    ]"
  >
    <span :class="hasToolChipClass ? 'truncate' : ''">{{ value }}</span>
  </span>
</template>
