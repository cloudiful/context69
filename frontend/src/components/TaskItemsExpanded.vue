<script setup lang="ts">
import { useI18n } from "vue-i18n";

import TaskItemAction from "./TaskItemAction.vue";
import type { ApiErrorSummary } from "../composables/use-error-toast";
import type { TaskItemResponse, TaskResponse } from "../services/api";

defineProps<{
  task: TaskResponse;
  items: TaskItemResponse[] | null | undefined;
  error: ApiErrorSummary | null | undefined;
  isLoading: boolean;
  isAdmin: boolean;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retryLoad: [taskId: string];
  retry: [task: TaskResponse];
  recover: [task: TaskResponse];
}>();

const { t } = useI18n();

function stageLabel(stage: string | null): string {
  return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage");
}

function itemSeverity(status: TaskItemResponse["status"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}

function externalJobLabel(job: TaskItemResponse["external_job"]): string {
  if (!job) return "--";
  return `${job.remote_task_id} · ${job.status}`;
}

function externalJobTitle(job: TaskItemResponse["external_job"]): string | undefined {
  if (!job) return undefined;
  const parts = [
    `${job.provider} ${job.remote_task_id}`,
    `status: ${job.remote_status ?? job.status}`,
    `submitted: ${job.submitted_at}`,
    job.last_polled_at ? `last polled: ${job.last_polled_at}` : null,
    job.deadline_at ? `deadline: ${job.deadline_at}` : null,
    job.error_message ? `error: ${job.error_message}` : null,
  ];
  return parts.filter(Boolean).join("\n");
}

function clampText(value: string | null | undefined, max: number): string | null {
  if (!value) return null;
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}
</script>

<template>
  <div class="p-3">
    <template v-if="items === undefined">
      <template v-if="error">
        <div class="flex flex-wrap items-center gap-2">
          <span
            class="text-sm text-(--ui-error)"
            :title="error.message || undefined"
            :aria-label="t('processingQueue.itemsLoadFailed')"
          >
            {{ t("processingQueue.itemsLoadFailed") }}<span
              v-if="error.status != null"
              class="ml-1 font-mono text-xs"
            >· {{ error.status }}</span>
          </span>
          <UButton
            color="neutral"
            variant="outline"
            size="sm"
            icon="i-lucide-refresh-cw"
            :loading="isLoading"
            :disabled="isLoading"
            :aria-label="t('processingQueue.retryLoadItems')"
            :title="t('processingQueue.retryLoadItems')"
            @click="emit('retryLoad', task.task_id)"
          />
        </div>
      </template>
      <div v-else class="text-sm text-muted">{{ t("common.loading") }}…</div>
    </template>
    <template v-else-if="items.length === 0">
      <div class="text-sm text-muted">{{ t("processingQueue.noItems") }}</div>
    </template>
    <div v-else class="grid gap-1">
      <div
        v-for="item in items"
        :key="item.item_id"
        class="grid grid-cols-[minmax(0,1fr)_auto_auto_minmax(0,1fr)_auto_auto_auto] items-center gap-3 rounded-md bg-surface-50 dark:bg-surface-900/40 px-3 py-1.5 text-sm"
      >
        <span class="block truncate font-mono text-xs text-muted" :title="item.item_id">{{ item.item_id }}</span>
        <UBadge :label="item.status" :color="itemSeverity(item.status)" variant="subtle" />
        <span class="whitespace-nowrap text-xs text-muted">{{ stageLabel(item.stage ?? null) }}</span>
        <span class="block truncate text-xs text-muted" :title="clampText(item.error_message, 240) || undefined">{{ item.error_message || "--" }}</span>
        <span class="block max-w-56 truncate text-xs text-muted" :title="externalJobTitle(item.external_job)">{{ externalJobLabel(item.external_job) }}</span>
        <span class="whitespace-nowrap text-xs text-muted">{{ t("processingQueue.attempts", { count: item.attempt_count }) }}</span>
        <TaskItemAction
          :item="item"
          :task="task"
          :is-admin="isAdmin"
          :is-acting="isActing"
          @retry="emit('retry', $event)"
          @recover="emit('recover', $event)"
        />
      </div>
    </div>
  </div>
</template>