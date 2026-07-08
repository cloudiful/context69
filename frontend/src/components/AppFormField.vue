<script setup lang="ts">
import FloatLabel from "primevue/floatlabel";

const props = withDefaults(defineProps<{
  floatLabel?: boolean;
  inputId: string;
  label: string;
  helper?: string;
  layout?: "stacked" | "inline";
}>(), {
  floatLabel: false,
  helper: "",
  layout: "stacked",
});
</script>

<template>
  <div
    class="app-form-field"
    :class="props.layout === 'inline'
      ? 'gap-2 md:grid-cols-[11rem_minmax(0,1fr)] md:items-center md:gap-x-4'
      : ''"
  >
    <label
      v-if="!props.floatLabel"
      class="app-form-field-label"
      :class="props.layout === 'inline' ? 'md:mb-0 md:self-center' : ''"
      :for="props.inputId"
    >
      {{ props.label }}
    </label>
    <div class="grid min-w-0 gap-2">
      <FloatLabel v-if="props.floatLabel" variant="on" class="app-float-field">
        <slot />
        <label class="app-float-field-label" :for="props.inputId">
          {{ props.label }}
        </label>
      </FloatLabel>
      <slot v-else />
      <p v-if="props.helper" class="app-form-field-help">{{ props.helper }}</p>
    </div>
  </div>
</template>
