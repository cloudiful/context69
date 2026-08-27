<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { TaskItemResponse, TaskResponse } from "../services/api";

// Per-item action affordance used inside the expanded task-items grid.
//
// `retry` emits when the failed item is retryable but not a Docling
// failure_stage (Qdrant/indexing/embedding/etc.) so the parent routes it
// through `recoverTask(task)` and the standard task-scoped retry endpoint.
// `recover` emits only for failed Docling items on admin sessions so the
// parent can call the admin `recoverDoclingFromItem(task)` helper which
// still reuses the same `actionTaskIds` guard to prevent duplicate row/cell
// requests for the same task.
const props = defineProps<{
  item: TaskItemResponse;
  task: TaskResponse;
  isAdmin: boolean;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [task: TaskResponse];
  recover: [task: TaskResponse];
}>();

const { t } = useI18n();

const failureStage = computed(() => props.item.failure_stage ?? null);
const isFailed = computed(() => props.item.status === "failed");
const isRetryable = computed(() => props.item.retryable === true && isFailed.value);
const isDoclingFailure = computed(
  () => failureStage.value === "docling" || failureStage.value === "docling_poll",
);

const canRecover = computed(() => props.isAdmin && isRetryable.value && isDoclingFailure.value);
const canRetry = computed(() => isRetryable.value && !canRecover.value);

const label = computed(() =>
  canRecover.value ? t("processingQueue.doclingRecovery") : t("processingQueue.retry"),
);

function handleClick() {
  if (canRecover.value) emit("recover", props.task);
  else if (canRetry.value) emit("retry", props.task);
}
</script>

<template>
  <UButton
    v-if="canRecover || canRetry"
    color="neutral"
    variant="ghost"
    size="sm"
    icon="i-lucide-rotate-ccw"
    :loading="isActing"
    :disabled="isActing"
    :label="label"
    :title="label"
    @click="handleClick"
  />
</template>