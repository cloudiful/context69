<script setup lang="ts">
import InputNumber from "primevue/inputnumber";
import SelectButton from "primevue/selectbutton";

import AppFormField from "./AppFormField.vue";

interface PresetOption {
  label: string;
  value: number;
}

const props = withDefaults(defineProps<{
  inputId: string;
  label: string;
  max?: number;
  min?: number;
  modelValue?: number | null;
  presetOptions: PresetOption[];
  presetTestId?: string;
  presetValue?: number | null;
  step?: number;
  testId?: string;
}>(), {
  max: undefined,
  min: undefined,
  modelValue: null,
  presetTestId: "",
  presetValue: null,
  step: 1,
  testId: "",
});

const emit = defineEmits<{
  "update:modelValue": [number | null];
  "update:presetValue": [number | null];
}>();
</script>

<template>
  <AppFormField :input-id="props.inputId" :label="props.label">
    <div class="app-number-preset-row">
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
      <SelectButton
        class="settings-preset-select app-number-preset-select"
        :model-value="props.presetValue"
        :data-testid="props.presetTestId || undefined"
        :options="props.presetOptions"
        option-label="label"
        option-value="value"
        :allow-empty="true"
        @update:model-value="emit('update:presetValue', typeof $event === 'number' ? $event : null)"
      />
    </div>
  </AppFormField>
</template>
