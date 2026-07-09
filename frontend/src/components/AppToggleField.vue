<script setup lang="ts">
import ToggleSwitch from "primevue/toggleswitch";

const props = defineProps<{
  inputId: string;
  label: string;
  helper?: string;
  helperInline?: boolean;
  modelValue: boolean;
  testId?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [boolean];
}>();

const togglePt = {
  root: { class: "h-6 w-11" },
  slider: { class: "rounded-full" },
  handle: { class: "h-[1.1rem] w-[1.1rem] mt-[-0.55rem] ml-[0.16rem]" },
} as const;
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
        <p class="text-sm font-medium text-app-text">{{ props.label }}</p>
        <p
          v-if="props.helper"
          class="text-xs leading-5 text-app-text-dim"
          :class="props.helperInline ? 'min-w-0' : ''"
        >
          {{ props.helper }}
        </p>
      </div>
    </div>
    <div class="shrink-0 self-center">
      <ToggleSwitch
        :input-id="props.inputId"
        :model-value="props.modelValue"
        :data-testid="props.testId"
        :pt="togglePt"
        @update:model-value="emit('update:modelValue', $event)"
      />
    </div>
  </label>
</template>
