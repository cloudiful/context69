<script setup lang="ts">
import ToggleSwitch, { type ToggleSwitchPassThroughMethodOptions } from "primevue/toggleswitch";

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

const pt = {
  root: "relative inline-flex h-5 w-9 shrink-0 align-middle",
  input: "peer absolute inset-0 z-10 m-0 h-full w-full cursor-pointer appearance-none opacity-0 disabled:cursor-not-allowed",
  slider: ({ context }: ToggleSwitchPassThroughMethodOptions) => ({
    class: [
      "absolute inset-0 rounded-full border transition-colors duration-150",
      "peer-focus-visible:ring-2 peer-focus-visible:ring-app-text/35 peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-app-bg",
      context.checked
        ? "border-emerald-400/70 bg-emerald-400"
        : "border-app-border-strong bg-app-surface-soft",
      context.disabled ? "opacity-45" : "",
    ],
  }),
  handle: ({ context }: ToggleSwitchPassThroughMethodOptions) => ({
    class: [
      "pointer-events-none absolute left-0.5 top-0.5 h-4 w-4 rounded-full shadow-sm transition-transform duration-150",
      context.checked
        ? "translate-x-4 bg-[#102019]"
        : "translate-x-0 bg-app-text-muted",
    ],
  }),
};
</script>

<template>
  <ToggleSwitch
    unstyled
    :input-id="inputId"
    :model-value="modelValue"
    :disabled="disabled"
    :data-testid="testId || undefined"
    :pt="pt"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>
