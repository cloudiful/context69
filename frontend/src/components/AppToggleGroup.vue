<script setup lang="ts">
import AppToggleField from "./AppToggleField.vue";

interface ToggleItem {
  helper?: string;
  inputId: string;
  key: string;
  label: string;
  testId?: string;
}

const props = withDefaults(defineProps<{
  columnsClass?: string;
  helperInline?: boolean;
  items: ToggleItem[];
  modelValue: Record<string, boolean>;
}>(), {
  columnsClass: "grid gap-4 lg:grid-cols-2 lg:items-start",
  helperInline: false,
});

const emit = defineEmits<{
  "update:modelValue": [Record<string, boolean>];
}>();

function updateItem(key: string, value: boolean) {
  emit("update:modelValue", {
    ...props.modelValue,
    [key]: value,
  });
}
</script>

<template>
  <div :class="props.columnsClass">
    <AppToggleField
      v-for="item in props.items"
      :key="item.key"
      :input-id="item.inputId"
      :label="item.label"
      :helper="item.helper"
      :helper-inline="props.helperInline"
      :model-value="!!props.modelValue[item.key]"
      :test-id="item.testId"
      @update:model-value="updateItem(item.key, $event)"
    />
  </div>
</template>
