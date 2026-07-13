<script setup lang="ts">
import AppFormField from "./AppFormField.vue";

const props = withDefaults(defineProps<{
  autocomplete?: string;
  disabled?: boolean;
  helper?: string;
  inputId: string;
  label: string;
  layout?: "stacked" | "inline";
  modelValue?: string | null;
  placeholder?: string;
  readonly?: boolean;
  testId?: string;
  type?: string;
}>(), {
  autocomplete: "",
  disabled: false,
  helper: "",
  modelValue: "",
  layout: "stacked",
  placeholder: "",
  readonly: false,
  testId: "",
  type: "text",
});

const emit = defineEmits<{
  "update:modelValue": [string];
}>();
</script>

<template>
  <AppFormField
    :input-id="props.inputId"
    :label="props.label"
    :helper="props.helper || undefined"
    :layout="props.layout"
  >
    <UInput
      :id="props.inputId"
      :model-value="props.modelValue ?? ''"
      :data-testid="props.testId || undefined"
      :type="props.type"
      :placeholder="props.placeholder"
      :autocomplete="props.autocomplete || undefined"
      :readonly="props.readonly"
      :disabled="props.disabled"
      @update:model-value="emit('update:modelValue', String($event ?? ''))"
    />
  </AppFormField>
</template>
