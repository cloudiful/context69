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

const items = computed<TabsItem[]>(() => [
  { label: t("processingQueue.tabs.processing"), value: "processing" },
  { label: t("processingQueue.tabs.completed"), value: "completed" },
  { label: t("processingQueue.tabs.trash"), value: "trash" },
]);

function handleUpdate(value: string | number) {
  if (value === "processing" || value === "completed" || value === "trash") {
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
