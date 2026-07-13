<script setup lang="ts">
import AppFormField from "./AppFormField.vue";

const props = withDefaults(defineProps<{
  helper?: string;
  inputId: string;
  label: string;
  layout?: "stacked" | "inline";
  max?: number;
  min?: number;
  modelValue?: number | null;
  step?: number;
  testId?: string;
}>(), {
  helper: "",
  layout: "stacked",
  max: undefined,
  min: undefined,
  modelValue: null,
  step: 1,
  testId: "",
});

const emit = defineEmits<{
  "update:modelValue": [number | null];
}>();
</script>

<template>
  <AppFormField
    :input-id="props.inputId"
    :label="props.label"
    :helper="props.helper || undefined"
    :layout="props.layout"
  >
    <UInputNumber
      :id="props.inputId"
      :model-value="props.modelValue"
      :data-testid="props.testId || undefined"
      :min="props.min"
      :max="props.max"
      :step="props.step"
      @update:model-value="emit('update:modelValue', typeof $event === 'number' ? $event : null)"
    />
  </AppFormField>
</template>
