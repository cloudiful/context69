<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { TaskItemResponse, TaskResponse } from "../services/api";

// The per-item action retries the whole task through the task-scoped endpoint
// (`POST /v1/tasks/{task_id}/retry`), which requeues every retryable item of
// that task — not just the item whose row the operator clicked. The label
// therefore states the real scope; calling it "Retry file" would promise a
// per-item effect the request does not have.
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
    :label="t('processingQueue.retryTaskScope')"
    :title="t('processingQueue.retryTaskScope')"
    data-testid="task-item-retry"
    @click="handleClick"
  />
</template>
