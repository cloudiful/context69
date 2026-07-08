<script setup lang="ts">
import ToggleSwitch from "primevue/toggleswitch";

const props = defineProps<{
  inputId: string;
  label: string;
  helper?: string;
  modelValue: boolean;
  testId?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [boolean];
}>();
</script>

<template>
  <label
    class="flex min-w-0 items-center justify-between gap-3"
    :for="props.inputId"
  >
    <div class="min-w-0 grow">
      <p class="text-sm font-medium text-app-text">{{ props.label }}</p>
      <p v-if="props.helper" class="text-xs leading-5 text-app-text-dim">{{ props.helper }}</p>
    </div>
    <div class="shrink-0 self-center">
      <ToggleSwitch
        :input-id="props.inputId"
        :model-value="props.modelValue"
        :data-testid="props.testId"
        @update:model-value="emit('update:modelValue', $event)"
      />
    </div>
  </label>
</template>
