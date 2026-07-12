<script setup lang="ts">
import ToggleSwitch from "primevue/toggleswitch";

withDefaults(defineProps<{
  disabled?: boolean;
  inputId: string;
  modelValue: boolean;
  testId?: string;
}>(), {
  disabled: false,
  testId: "",
});

const emit = defineEmits<{
  "update:modelValue": [boolean];
}>();

</script>

<template>
  <ToggleSwitch
    :input-id="inputId"
    :model-value="modelValue"
    :disabled="disabled"
    :data-testid="testId || undefined"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>
