<script setup lang="ts">
import Select from "primevue/select";

import AppFormField from "./AppFormField.vue";

const props = withDefaults(defineProps<{
  floatLabel?: boolean;
  helper?: string;
  inputId: string;
  label: string;
  layout?: "stacked" | "inline";
  modelValue?: string | number | null;
  optionLabel?: string;
  optionValue?: string;
  options: Array<Record<string, unknown>>;
  placeholder?: string;
  testId?: string;
}>(), {
  floatLabel: false,
  helper: "",
  layout: "stacked",
  modelValue: null,
  optionLabel: "label",
  optionValue: "value",
  placeholder: "",
  testId: "",
});

const emit = defineEmits<{
  "update:modelValue": [string | number | null];
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
    <Select
      :input-id="props.inputId"
      :model-value="props.modelValue ?? null"
      :data-testid="props.testId || undefined"
      :options="props.options"
      :option-label="props.optionLabel"
      :option-value="props.optionValue"
      :placeholder="props.placeholder || undefined"
      @update:model-value="emit('update:modelValue', $event ?? null)"
    />
  </AppFormField>
</template>
