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
  <label class="app-toggle-field" :for="props.inputId">
    <div class="min-w-0">
      <p class="app-toggle-field-label">{{ props.label }}</p>
      <p v-if="props.helper" class="app-toggle-field-help">{{ props.helper }}</p>
    </div>
    <div class="app-toggle-control">
      <ToggleSwitch
        :input-id="props.inputId"
        :model-value="props.modelValue"
        :data-testid="props.testId"
        @update:model-value="emit('update:modelValue', $event)"
      />
    </div>
  </label>
</template>
