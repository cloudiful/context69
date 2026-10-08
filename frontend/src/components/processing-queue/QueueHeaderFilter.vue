<script setup lang="ts">
import { computed } from "vue";
import type { DropdownMenuItem } from "@nuxt/ui";

export interface QueueHeaderFilterOption {
  label: string;
  value: string | null;
  /** Nested submenu entries. A parent opens its submenu instead of selecting. */
  children?: QueueHeaderFilterOption[];
}

const props = defineProps<{
  // Accessible name of the filter, e.g. the column's filter label.
  label: string;
  options: QueueHeaderFilterOption[];
  selected: string | null;
  // Marks the button active even when the paired filter carries the selection
  // (the waiting-reason filter also owns the dependency narrowing).
  active?: boolean;
}>();

const emit = defineEmits<{
  select: [value: string | null, group?: string | null];
}>();

const isActive = computed(() => props.active ?? props.selected !== null);

// A child of a submenu reports its own value plus the parent reason it belongs
// to, so one filter control can set both without a second button.
function toItem(option: QueueHeaderFilterOption, group: string | null = null): DropdownMenuItem {
  if (option.children?.length) {
    return {
      label: option.label,
      children: option.children.map((child) => toItem(child, option.value)),
    };
  }
  return {
    label: option.label,
    // The leading check is the only selection affordance: it keeps the menu
    // item semantics plain while still naming the active value.
    icon: option.value === props.selected && group === null ? "i-lucide-check" : undefined,
    class: option.value === props.selected ? "text-primary" : undefined,
    onSelect: () => choose(option, group),
  };
}

const menuItems = computed(() => props.options.map((option) => toItem(option)));

// Selecting the active value only closes the menu; the caller (and the queue's
// setFilter guard) sends no duplicate request for it.
function choose(option: QueueHeaderFilterOption, group: string | null) {
  if (option.value === props.selected && group === null) return;
  emit("select", option.value, group);
}
</script>

<template>
  <UDropdownMenu
    :items="menuItems"
    :content="{ align: 'start', 'aria-label': props.label }"
  >
    <UButton
      variant="ghost"
      color="neutral"
      size="xs"
      icon="i-lucide-filter"
      :class="isActive ? 'text-primary' : 'text-muted'"
      :aria-label="props.label"
      :aria-pressed="isActive"
      :data-active="isActive"
    />
  </UDropdownMenu>
</template>