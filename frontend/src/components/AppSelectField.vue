<script setup lang="ts">
import AppFormField from "./AppFormField.vue";

const props = withDefaults(defineProps<{
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
    :input-id="props.inputId"
    :label="props.label"
    :helper="props.helper || undefined"
    :layout="props.layout"
  >
    <USelect
      :id="props.inputId"
      :model-value="props.modelValue ?? null"
      :data-testid="props.testId || undefined"
      :items="props.options"
      :label-key="props.optionLabel"
      :value-key="props.optionValue"
      :placeholder="props.placeholder || undefined"
      @update:model-value="emit('update:modelValue', $event ?? null)"
    />
  </AppFormField>
</template>
