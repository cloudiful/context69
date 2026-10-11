<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import TaskItemAction from "../TaskItemAction.vue";
import TaskItemProgress from "./TaskItemProgress.vue";
import { pipelineProgressStageLabel } from "./task-item-progress";
import { queueStageLabel } from "../../composables/queue-helpers";
import type { TaskItemResponse, TaskResponse } from "../../services/api";

// One item's row inside the expanded task panel.
//
// The panel is progress-first: the first column carries what the worker is
// actually doing, and the second carries the operator's levers. On a wide
// screen the two sit side by side; on a narrow one they stack, because a fixed
// two-column layout would either clip the counters or force a horizontal
// scroll inside a row the operator is already reading.
//
// Identifiers and attempt bookkeeping stay behind a disclosure. They are real
// diagnostics, but they are not what an operator reads first, and repeating the
// parent row's type/group/timestamps here would add nothing the row does not
// already say.
const props = defineProps<{
  item: TaskItemResponse;
  task: TaskResponse;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [];
}>();

const { t } = useI18n();

const hasProgress = computed(() => props.item.progress != null);
// Two different vocabularies, deliberately: the progress stage is the stage the
// item actually reached, while the item's persisted stage column uses the
// queue's own stage filter vocabulary. Both render through their own locale
// keys so neither leaks a raw identifier or a raw i18n key.
const progressStageLabel = computed(() =>
  pipelineProgressStageLabel(t, props.item.progress?.stage ?? null),
);
const persistedStageLabel = computed(() => queueStageLabel(t, props.item.stage ?? null));

// Identity for a multi-item list. The ordinal is the part that actually
// distinguishes two items of the same task, and the readable name is what
// makes the row recognizable at a glance.
const subject = computed(() => {
  const name = props.item.file_name?.trim() || props.item.document_title?.trim();
  return name || t("processingQueue.details.positionValue", { position: props.item.ordinal });
});

// Mirrors the shared action's own rule so this row never reserves action space
// for a control it will not render.
const isRetryable = computed(
  () => props.item.retryable === true && props.item.status === "failed",
);

const KNOWN_ITEM_STATUSES: ReadonlySet<string> = new Set<TaskItemResponse["status"]>([
  "queued",
  "running",
  "waiting",
  "succeeded",
  "failed",
  "cancelled",
]);

function statusLabel(status: TaskItemResponse["status"]): string {
  return KNOWN_ITEM_STATUSES.has(status) ? t(`processingQueue.statuses.${status}`) : status;
}

function itemSeverity(status: TaskItemResponse["status"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}
</script>

<template>
  <article
    class="rounded-md bg-surface-50 p-3 text-sm dark:bg-surface-900/40"
    data-testid="task-item-row"
  >
    <div class="grid min-w-0 gap-3 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
      <!-- Progress first: the useful half of the panel. -->
      <div class="flex min-w-0 flex-col gap-1.5">
        <div class="flex flex-wrap items-center gap-2">
          <UBadge
            :label="statusLabel(item.status)"
            :color="itemSeverity(item.status)"
            variant="subtle"
            data-testid="task-item-status"
          />
          <span class="min-w-0 truncate text-xs text-muted" :title="subject" data-testid="task-item-subject">
            {{ subject }}
          </span>
        </div>
        <TaskItemProgress v-if="hasProgress" :progress="item.progress" />
        <p
          v-else-if="item.stage"
          class="text-xs text-muted"
          data-testid="task-item-stage"
        >{{ persistedStageLabel }}</p>
      </div>

      <!-- Context and actions: why the item is not moving, and what to do. -->
      <div class="flex min-w-0 flex-col gap-1.5">
        <p
          v-if="item.error_message"
          class="min-w-0 break-words text-xs text-(--ui-error)"
          :title="item.error_message"
          data-testid="task-item-error"
        >{{ item.error_message }}</p>
        <p
          v-else-if="item.waiting_reason"
          class="min-w-0 break-words text-xs text-(--ui-warning)"
          data-testid="task-item-waiting"
        >{{ t(`processingQueue.waitingReasons.${item.waiting_reason}`) }}</p>

        <TaskItemAction
          v-if="isRetryable"
          :item="item"
          :task="task"
          :is-acting="isActing"
          @retry="emit('retry')"
        />
      </div>
    </div>

    <!-- Technical identifiers and attempt bookkeeping: real diagnostics, but
         secondary to what the item is doing right now. -->
    <details class="mt-1.5">
      <summary class="cursor-pointer text-xs text-muted" data-testid="task-item-technical-toggle">
        {{ t("processingQueue.details.technical") }}
      </summary>
      <dl
        class="grid gap-x-3 gap-y-0.5 pt-1 text-xs sm:grid-cols-[minmax(5.5rem,auto)_minmax(0,1fr)]"
        data-testid="task-item-detail"
      >
        <dt class="text-muted">{{ t("processingQueue.details.itemId") }}</dt>
        <dd class="min-w-0 break-all font-mono text-muted">{{ item.item_id }}</dd>
        <dt class="text-muted">{{ t("processingQueue.details.position") }}</dt>
        <dd class="min-w-0 text-muted">{{ item.ordinal }}</dd>
        <dt class="text-muted">{{ t("processingQueue.details.stage") }}</dt>
        <dd class="min-w-0 text-muted">{{ persistedStageLabel }}</dd>
        <dt class="text-muted">{{ t("processingQueue.details.attempts") }}</dt>
        <dd class="min-w-0 text-muted">{{ item.attempt_count }}</dd>
      </dl>
    </details>
  </article>
</template>