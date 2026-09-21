<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { TaskItemResponse, TaskResponse } from "../services/api";

// Per-item action affordance used inside the expanded task-items grid.
//
// A failed retryable item emits `retry`; the parent routes it through
// `recoverTask(task)` and the task-scoped retry endpoint. Issue 529 removed
// the Docling recovery path (the async submit/poll chain no longer exists),
// so retry is the single per-item action.
const props = defineProps<{
  item: TaskItemResponse;
  task: TaskResponse;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [task: TaskResponse];
}>();

const { t } = useI18n();

const isRetryable = computed(() => props.item.retryable === true && props.item.status === "failed");

function handleClick() {
  if (isRetryable.value) emit("retry", props.task);
}
</script>

<template>
  <UButton
    v-if="isRetryable"
    color="neutral"
    variant="ghost"
    size="sm"
    icon="i-lucide-rotate-ccw"
    :loading="isActing"
    :disabled="isActing"
    :label="t('processingQueue.retry')"
    :title="t('processingQueue.retry')"
    @click="handleClick"
  />
</template>
