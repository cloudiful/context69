<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  inputId: string;
  label: string;
  helper?: string;
  helperInline?: boolean;
  modelValue: boolean;
  name?: string;
  testId?: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [boolean];
}>();

// USwitch renders the switch before the label wrapper, so the container is
// ordered last to keep the previous label-left / switch-right settings layout.
const switchUi = computed(() => ({
  root: props.helperInline
    ? "flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1"
    : "flex min-w-0 items-start justify-between gap-3",
  container: "order-last shrink-0 self-center",
  wrapper: props.helperInline
    ? "ms-0 flex min-w-0 grow flex-wrap items-baseline gap-x-2 gap-y-1"
    : "ms-0 min-w-0 grow",
}));
</script>

<template>
  <USwitch
    :id="props.inputId"
    :name="props.name || props.inputId"
    :model-value="props.modelValue"
    :data-testid="props.testId"
    :disabled="props.disabled"
    class="app-toggle-field w-full"
    :label="props.label"
    :description="props.helper || undefined"
    :ui="switchUi"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>
