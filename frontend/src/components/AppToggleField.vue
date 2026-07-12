<script setup lang="ts">
import AppSwitch from "./AppSwitch.vue";

const props = defineProps<{
  inputId: string;
  label: string;
  helper?: string;
  helperInline?: boolean;
  modelValue: boolean;
  testId?: string;
  disabled?: boolean;
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
      <div
        :class="props.helperInline
          ? 'flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1'
          : ''"
      >
        <p class="text-sm font-medium text-color">{{ props.label }}</p>
        <p
          v-if="props.helper"
          class="text-xs leading-5 text-muted-color"
          :class="props.helperInline ? 'min-w-0' : ''"
        >
          {{ props.helper }}
        </p>
      </div>
    </div>
    <div class="shrink-0 self-center">
      <AppSwitch
        :input-id="props.inputId"
        :model-value="props.modelValue"
        :data-testid="props.testId"
        :disabled="props.disabled"
        @update:model-value="emit('update:modelValue', $event)"
      />
    </div>
  </label>
</template>
