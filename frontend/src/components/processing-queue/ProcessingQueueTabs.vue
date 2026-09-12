<script setup lang="ts">
import { computed } from "vue";
import type { TabsItem } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

type QueueTab = "processing" | "completed" | "trash";

const props = defineProps<{
  modelValue: QueueTab;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: QueueTab];
}>();

const { t } = useI18n();

// Trash stays a disabled placeholder until the backend exposes deleted-task
// history; the badge makes the unavailability explicit instead of implying a
// working recycle bin.
const items = computed<TabsItem[]>(() => [
  { label: t("processingQueue.tabs.processing"), value: "processing" },
  { label: t("processingQueue.tabs.completed"), value: "completed" },
  {
    label: t("processingQueue.tabs.trash"),
    value: "trash",
    disabled: true,
    badge: t("processingQueue.tabs.trashBadge"),
  },
]);

function handleUpdate(value: string | number) {
  if (value === "processing" || value === "completed") {
    emit("update:modelValue", value);
  }
}
</script>

<template>
  <UTabs
    :model-value="props.modelValue"
    :items="items"
    :content="false"
    data-testid="processing-queue-tabs"
    @update:model-value="handleUpdate"
  />
</template>
