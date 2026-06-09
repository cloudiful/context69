<script setup lang="ts">

interface ToggleItem {
  helper?: string;
  inputId: string;
  key: string;
  label: string;
  testId?: string;
}

const props = withDefaults(defineProps<{
  columnsClass?: string;
  items: ToggleItem[];
  modelValue: Record<string, boolean>;
}>(), {
  columnsClass: "grid gap-4 lg:grid-cols-2 lg:items-start",
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
    <button
      v-for="item in props.items"
      :key="item.key"
      :id="item.inputId"
      :data-testid="item.testId"
      class="app-toggle-chip"
      :class="{ 'app-toggle-chip-active': !!props.modelValue[item.key] }"
      type="button"
      :aria-pressed="!!props.modelValue[item.key]"
      @click="updateItem(item.key, !props.modelValue[item.key])"
    >
      <span class="app-toggle-chip-label">{{ item.label }}</span>
      <span v-if="item.helper" class="app-toggle-chip-help">{{ item.helper }}</span>
    </button>
  </div>
</template>
