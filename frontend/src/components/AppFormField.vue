<script setup lang="ts">
const props = withDefaults(defineProps<{
  inputId: string;
  label: string;
  helper?: string;
  layout?: "stacked" | "inline";
}>(), {
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
      class="app-form-field-label"
      :class="props.layout === 'inline' ? 'md:mb-0 md:self-center' : ''"
      :for="props.inputId"
    >
      {{ props.label }}
    </label>
    <div class="grid min-w-0 gap-2">
      <slot />
      <p v-if="props.helper" class="app-form-field-help">{{ props.helper }}</p>
    </div>
  </div>
</template>
