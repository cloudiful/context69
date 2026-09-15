<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import TaskItemAction from "./TaskItemAction.vue";
import { apiClient, type TaskItemResponse, type TaskItemStatus, type TaskResponse } from "../services/api";
import { summarizeApiError, type ApiErrorSummary } from "../composables/use-error-toast";

const props = defineProps<{
  task: TaskResponse;
  isAdmin: boolean;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [task: TaskResponse];
  recover: [task: TaskResponse];
}>();

const { t } = useI18n();

// Page size matches the previous single-fetch limit so existing callers and
// tests asserting `{ limit: 100 }` stay valid. Follow `next_cursor` until null
// to walk the full filtered set (2781-scale); the backend pins active-first
// order (failed/running first, succeeded last) so no client sort is needed.
const ITEMS_PAGE_SIZE = 100;
const ACTIVE_ITEM_REFRESH_STATUSES: TaskResponse["status"][] = ["queued", "running", "waiting"];

const statusFilter = ref<TaskItemStatus | null>(null);
const items = ref<TaskItemResponse[]>([]);
const nextCursor = ref<string | null>(null);
const isLoading = ref(false);
const isLoadingMore = ref(false);
const error = ref<ApiErrorSummary | null>(null);
let requestId = 0;

const hasMore = computed(() => nextCursor.value != null);
const shownCount = computed(() => items.value.length);
const totalCount = computed(() => props.task.progress.total);

const statusOptions = computed(() => [
  { label: t("processingQueue.allItemStatuses"), value: null },
  ...(["queued", "running", "waiting", "succeeded", "failed", "cancelled"] as TaskItemStatus[]).map((value) => ({
    label: t(`processingQueue.statuses.${value}`),
    value,
  })),
]);

async function loadFirstPage() {
  const current = ++requestId;
  isLoading.value = true;
  error.value = null;
  try {
    const response = await apiClient.getTaskItems(props.task.task_id, {
      limit: ITEMS_PAGE_SIZE,
      status: statusFilter.value,
    });
    if (current !== requestId) return;
    items.value = response.items;
    nextCursor.value = response.next_cursor ?? null;
  } catch (loadError) {
    if (current !== requestId) return;
    items.value = [];
    nextCursor.value = null;
    error.value = summarizeApiError(loadError, 240);
  } finally {
    if (current === requestId) isLoading.value = false;
  }
}

async function loadMore() {
  if (isLoading.value || isLoadingMore.value || nextCursor.value == null) return;
  const current = ++requestId;
  isLoadingMore.value = true;
  try {
    const response = await apiClient.getTaskItems(props.task.task_id, {
      limit: ITEMS_PAGE_SIZE,
      cursor: nextCursor.value,
      status: statusFilter.value,
    });
    if (current !== requestId) return;
    items.value = [...items.value, ...response.items];
    nextCursor.value = response.next_cursor ?? null;
  } catch {
    // Keep the cursor so the user can retry: a failed page appends nothing.
  } finally {
    if (current === requestId) isLoadingMore.value = false;
  }
}

function handleFilterChange(value: TaskItemStatus | null) {
  if (statusFilter.value === value) return;
  // Cursor is scoped to the filter: reset to no cursor when it changes.
  statusFilter.value = value;
}

watch(statusFilter, () => {
  nextCursor.value = null;
  void loadFirstPage();
});

watch(() => props.task.task_id, () => {
  statusFilter.value = null;
  items.value = [];
  nextCursor.value = null;
  void loadFirstPage();
});

// Live refresh for active tasks: the queue SSE stream (3s per-task coalesced
// server-side) bumps `task.updated_at` via the in-place merge. Reset to the
// first page so the active-first pinning stays fresh; the user can page again
// for the remainder. Inactive tasks never auto-refresh.
watch(() => props.task.updated_at, (next, prev) => {
  if (next === prev) return;
  if (!ACTIVE_ITEM_REFRESH_STATUSES.includes(props.task.status)) return;
  if (isLoading.value || isLoadingMore.value) return;
  if (items.value.length === 0 && !error.value) return;
  void loadFirstPage();
});

onMounted(() => {
  void loadFirstPage();
});

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

type ExternalJob = NonNullable<TaskItemResponse["external_job"]>;

function isPlaceholderRemoteId(remoteTaskId: string): boolean {
  return remoteTaskId.startsWith("submitting-");
}

function externalJobKind(job: ExternalJob): "uncertain" | "quarantined" | "queued" | "running" | "failed" | "done" | "other" {
  if (job.status === "submitting") return "uncertain";
  if (job.status === "orphaned") return "quarantined";
  if (job.status === "pending") return "queued";
  if (job.status === "running") return "running";
  if (job.status === "failed" || job.status === "error") return "failed";
  if (job.status === "succeeded" || job.status === "success" || job.status === "complete" || job.status === "completed") return "done";
  return "other";
}

function externalJobLabel(job: TaskItemResponse["external_job"]): string {
  if (!job) return "--";
  const kind = externalJobKind(job);
  if (kind === "uncertain") return t("processingQueue.remoteStatuses.uncertain");
  if (kind === "quarantined") return t("processingQueue.remoteStatuses.quarantined");
  if (kind === "queued") return t("processingQueue.remoteStatuses.doclingQueued");
  if (kind === "running") return t("processingQueue.remoteStatuses.doclingRunning");
  if (kind === "failed") return t("processingQueue.remoteStatuses.doclingFailed");
  if (kind === "done") return t("processingQueue.remoteStatuses.doclingDone");
  return job.remote_status ?? job.status;
}

function externalJobSeverity(job: TaskItemResponse["external_job"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (!job) return "neutral";
  const kind = externalJobKind(job);
  if (kind === "done") return "success";
  if (kind === "failed") return "error";
  if (kind === "running") return "primary";
  if (kind === "uncertain" || kind === "queued") return "warning";
  return "neutral";
}

function externalJobTitle(job: TaskItemResponse["external_job"]): string | undefined {
  if (!job) return undefined;
  const kind = externalJobKind(job);
  const parts = [
    `${job.provider} ${job.remote_task_id}`,
    `status: ${job.remote_status ?? job.status}`,
    kind === "uncertain"
      ? (isPlaceholderRemoteId(job.remote_task_id)
        ? t("processingQueue.remoteStatuses.uncertainHintPlaceholder")
        : t("processingQueue.remoteStatuses.uncertainHint"))
      : null,
    kind === "quarantined" ? t("processingQueue.remoteStatuses.quarantinedHint") : null,
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
    <div class="mb-2 flex flex-wrap items-center gap-2">
      <span class="text-xs text-muted" data-testid="task-items-count">
        {{ t("processingQueue.itemsCount", { shown: shownCount, total: totalCount }) }}
      </span>
      <USelect
        :model-value="statusFilter"
        :items="statusOptions"
        value-key="value"
        size="sm"
        class="w-36"
        data-testid="task-items-filter"
        :aria-label="t('processingQueue.itemStatusFilter')"
        @update:model-value="handleFilterChange($event as TaskItemStatus | null)"
      />
    </div>
    <template v-if="isLoading">
      <div class="text-sm text-muted">{{ t("common.loading") }}…</div>
    </template>
    <template v-else-if="error && items.length === 0">
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
          @click="loadFirstPage"
        />
      </div>
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
        <UBadge
          v-if="item.external_job"
          :label="externalJobLabel(item.external_job)"
          :color="externalJobSeverity(item.external_job)"
          variant="subtle"
          :title="externalJobTitle(item.external_job)"
        />
        <span v-else class="text-xs text-muted">--</span>
        <span class="whitespace-nowrap text-xs text-muted">{{ t("processingQueue.attempts", { count: item.attempt_count }) }}</span>
        <TaskItemAction
          :item="item"
          :task="props.task"
          :is-admin="props.isAdmin"
          :is-acting="props.isActing"
          @retry="emit('retry', $event)"
          @recover="emit('recover', $event)"
        />
      </div>
      <div v-if="hasMore" class="mt-1 flex justify-center">
        <UButton
          color="neutral"
          variant="outline"
          size="sm"
          icon="i-lucide-chevron-down"
          data-testid="task-items-load-more"
          :loading="isLoadingMore"
          :disabled="isLoadingMore"
          :label="t('processingQueue.loadMoreItems')"
          @click="loadMore"
        />
      </div>
    </div>
  </div>
</template>
