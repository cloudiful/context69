<script setup lang="ts">
import InputNumber from "primevue/inputnumber";

import AppFormField from "./AppFormField.vue";

const props = withDefaults(defineProps<{
  floatLabel?: boolean;
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
  floatLabel: false,
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
    :float-label="props.floatLabel"
    :input-id="props.inputId"
    :label="props.label"
    :helper="props.helper || undefined"
    :layout="props.layout"
  >
    <InputNumber
      :input-id="props.inputId"
      :model-value="props.modelValue"
      :data-testid="props.testId || undefined"
      :min="props.min"
      :max="props.max"
      :step="props.step"
      :use-grouping="false"
      @update:model-value="emit('update:modelValue', typeof $event === 'number' ? $event : null)"
    />
  </AppFormField>
</template>
