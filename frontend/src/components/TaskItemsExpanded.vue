<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import TaskDiagnosePanel from "./TaskDiagnosePanel.vue";
import TaskItemAction from "./TaskItemAction.vue";
import { apiClient, type TaskItemResponse, type TaskItemStatus, type TaskResponse } from "../services/api";
import { queueStageLabel, taskDocumentTitle, taskFileName } from "../composables/queue-helpers";
import { summarizeApiError, type ApiErrorSummary } from "../composables/use-error-toast";
import { formatTimestamp } from "../utils/format";
import { libraryDependencyLabel } from "../utils/library-status";

const props = defineProps<{
  task: TaskResponse;
  isActing: boolean;
}>();

const emit = defineEmits<{
  retry: [task: TaskResponse];
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
    error.value = summarizeApiError(loadError);
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
  return queueStageLabel(t, stage);
}

// Structured detail panel (issue 723): the task's identifiers and lifecycle
// fields live here, not in the queue row, so a row can be read by subject
// first and the diagnostics stay one click away.
const taskDetailRows = computed(() => {
  const task = props.task;
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: t("processingQueue.details.taskId"), value: task.task_id, mono: true },
    { label: t("processingQueue.details.type"), value: t(`processingQueue.kinds.${task.kind}`) },
    { label: t("processingQueue.details.group"), value: task.group_path || task.source_key || "--" },
  ];
  // The collapsed row already shows the file name and title, so the panel adds
  // them only when the task carries one.
  if (taskFileName(task)) {
    rows.push({ label: t("processingQueue.details.fileName"), value: taskFileName(task)! });
  }
  if (taskDocumentTitle(task)) {
    rows.push({
      label: t("processingQueue.details.documentTitle"),
      value: taskDocumentTitle(task)!,
    });
  }
  rows.push(
    { label: t("processingQueue.details.createdAt"), value: formatTimestamp(task.created_at) },
    { label: t("processingQueue.details.updatedAt"), value: formatTimestamp(task.updated_at) },
  );
  if (task.finished_at) {
    rows.push({ label: t("processingQueue.details.finishedAt"), value: formatTimestamp(task.finished_at) });
  }
  if (task.dependency_key) {
    rows.push({ label: t("processingQueue.details.dependency"), value: libraryDependencyLabel(t, task.dependency_key) });
  }
  return rows;
});

function itemDetailRows(item: TaskItemResponse) {
  const rows: { label: string; value: string; mono?: boolean }[] = [
    { label: t("processingQueue.details.itemId"), value: item.item_id, mono: true },
    { label: t("processingQueue.details.position"), value: String(item.ordinal) },
    { label: t("processingQueue.details.stage"), value: stageLabel(item.stage ?? null) },
  ];
  // The file name and document title are the item's own readable context; they
  // stay absent while the item has neither, so the panel never shows a blank.
  if (item.file_name) rows.push({ label: t("processingQueue.details.fileName"), value: item.file_name });
  if (item.document_title) {
    rows.push({ label: t("processingQueue.details.documentTitle"), value: item.document_title });
  }
  if (item.file_id) rows.push({ label: t("processingQueue.details.fileId"), value: item.file_id, mono: true });
  if (item.dependency_key) {
    rows.push({ label: t("processingQueue.details.dependency"), value: libraryDependencyLabel(t, item.dependency_key) });
  }
  rows.push({ label: t("processingQueue.details.attempts"), value: String(item.attempt_count) });
  if (item.error_message) rows.push({ label: t("processingQueue.details.error"), value: item.error_message });
  return rows;
}

function itemSeverity(status: TaskItemResponse["status"]): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}

// Item statuses are backend enum identifiers; the row badge renders the locale
// key so zh-CN never shows raw English. The raw value is kept only as the
// fallback for a value this build does not yet know.
const KNOWN_ITEM_STATUSES: ReadonlySet<string> = new Set<TaskItemStatus>([
  "queued",
  "running",
  "waiting",
  "succeeded",
  "failed",
  "cancelled",
]);

function itemStatusLabel(status: TaskItemResponse["status"]): string {
  return KNOWN_ITEM_STATUSES.has(status) ? t(`processingQueue.statuses.${status}`) : status;
}
</script>

<template>
  <div class="flex flex-col gap-3 p-3">
    <TaskDiagnosePanel :task-id="props.task.task_id" />
    <!-- Structured detail panel (issue 723): the task's identifiers and
         lifecycle fields live here, not in the queue row, so a row is read by
         subject first and the diagnostics stay one click away. -->
    <dl
      class="grid gap-x-4 gap-y-1 rounded-md bg-surface-50 px-3 py-2 text-xs dark:bg-surface-900/40 sm:grid-cols-[minmax(6rem,auto)_minmax(0,1fr)]"
      data-testid="task-detail-panel"
    >
      <dt class="col-span-full text-xs font-medium text-muted">{{ t("processingQueue.details.title") }}</dt>
      <template v-for="row in taskDetailRows" :key="row.label">
        <dt class="text-muted">{{ row.label }}</dt>
        <dd class="min-w-0 break-all text-(--ui-text)" :class="{ 'font-mono': row.mono }">{{ row.value }}</dd>
      </template>
    </dl>
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
    <div v-else class="flex flex-col gap-1.5">
      <article
        v-for="item in items"
        :key="item.item_id"
        class="flex flex-col gap-1 rounded-md bg-surface-50 px-3 py-2 text-sm dark:bg-surface-900/40"
        data-testid="task-item-row"
      >
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <UBadge :label="itemStatusLabel(item.status)" :color="itemSeverity(item.status)" variant="subtle" data-testid="task-item-status" />
          <span class="whitespace-nowrap text-xs text-muted">{{ stageLabel(item.stage ?? null) }}</span>
          <span class="whitespace-nowrap text-xs text-muted">{{ t("processingQueue.attempts", { count: item.attempt_count }) }}</span>
          <span class="min-w-0 flex-1 truncate text-xs text-muted" :title="item.error_message || undefined">{{ item.error_message || "--" }}</span>
          <TaskItemAction
            :item="item"
            :task="props.task"
            :is-acting="props.isActing"
            @retry="emit('retry', $event)"
          />
        </div>
        <!-- Identifiers and lifecycle fields are details, not the row headline:
             a definition list keeps them readable and selectable instead of a
             flat dump of mono columns. -->
        <dl class="grid gap-x-3 gap-y-0.5 pl-1 text-xs sm:grid-cols-[minmax(5.5rem,auto)_minmax(0,1fr)]" data-testid="task-item-detail">
          <template v-for="row in itemDetailRows(item)" :key="row.label">
            <dt class="text-muted">{{ row.label }}</dt>
            <dd class="min-w-0 break-all text-muted" :class="{ 'font-mono': row.mono }">{{ row.value }}</dd>
          </template>
        </dl>
      </article>
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
