<script setup lang="ts">
import { ref } from "vue";

export interface QueueHeaderFilterOption {
  label: string;
  value: string | null;
}

const props = defineProps<{
  // Accessible name of the filter, e.g. the column's filter label.
  label: string;
  options: QueueHeaderFilterOption[];
  selected: string | null;
}>();

const emit = defineEmits<{
  select: [value: string | null];
}>();

const open = ref(false);

function choose(value: string | null, close: () => void) {
  // Selecting the active value only closes the popover; the caller (and the
  // queue's setFilter guard) sends no duplicate request for it.
  if (value !== props.selected) {
    emit("select", value);
  }
  close();
}
</script>

<template>
  <UPopover v-model:open="open">
    <UButton
      variant="ghost"
      color="neutral"
      size="xs"
      icon="i-lucide-filter"
      :class="props.selected !== null ? 'text-primary' : 'text-muted'"
      :aria-label="props.label"
      :aria-pressed="props.selected !== null"
      :data-active="props.selected !== null"
    />
    <template #content="{ close }">
      <div role="listbox" :aria-label="props.label" class="flex min-w-40 flex-col gap-0.5 p-1.5">
        <UButton
          v-for="option in props.options"
          :key="option.label"
          variant="ghost"
          color="neutral"
          size="sm"
          class="justify-start"
          :class="option.value === props.selected ? 'text-primary' : ''"
          :aria-selected="option.value === props.selected"
          role="option"
          @click="choose(option.value, close)"
        >
          <span class="min-w-0 flex-1 truncate text-left">{{ option.label }}</span>
          <UIcon v-if="option.value === props.selected" name="i-lucide-check" class="size-4 shrink-0" />
        </UButton>
      </div>
    </template>
  </UPopover>
</template>
