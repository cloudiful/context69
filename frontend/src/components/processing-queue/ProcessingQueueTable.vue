<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import TaskItemsExpanded from "../TaskItemsExpanded.vue";
import { apiClient, type TaskItemResponse, type TaskResponse, type TaskSortBy, type TaskStatus } from "../../services/api";
import { summarizeApiError, type ApiErrorSummary } from "../../composables/use-error-toast";
import { formatTimestamp } from "../../utils/format";
import { libraryDependencyLabel } from "../../utils/library-status";

const props = defineProps<{
  items: TaskResponse[];
  loading: boolean;
  isAdmin: boolean;
  isActing: (task: TaskResponse) => boolean;
  isRecoverableTask: (task: TaskResponse) => boolean;
  isDoclingRecoveryTask: (task: TaskResponse) => boolean;
}>();

const emit = defineEmits<{
  recover: [task: TaskResponse];
  recoverItem: [task: TaskResponse];
  cancel: [task: TaskResponse];
  sort: [value: { field: TaskSortBy; direction: "asc" | "desc" } | null];
}>();

const { t } = useI18n();

// Bound the upstream message that flows into a tooltip so a noisy error
// string cannot blow up the layout. The localized label below is still the
// primary text; the tooltip only carries the bounded detail when present.
const ITEM_ERROR_TOOLTIP_MAX = 240;
const SORTABLE_FIELDS: TaskSortBy[] = ["kind", "group_path", "status", "stage", "updated_at"];

const expandedRows = ref<Record<string, boolean>>({});
const expandedItems = ref<Record<string, TaskItemResponse[] | undefined>>({});
const expandedError = ref<Record<string, ApiErrorSummary | null>>({});
const expandingTaskId = ref<string | null>(null);
const sorting = ref<{ id: string; desc: boolean }[]>([]);

watch(sorting, (value) => {
  const next = value[0];
  if (!next) {
    emit("sort", null);
    return;
  }
  if (!SORTABLE_FIELDS.includes(next.id as TaskSortBy)) return;
  emit("sort", { field: next.id as TaskSortBy, direction: next.desc ? "desc" : "asc" });
});

async function loadTaskItems(taskId: string) {
  expandingTaskId.value = taskId;
  try {
    const response = await apiClient.getTaskItems(taskId, { limit: 100 });
    expandedItems.value = { ...expandedItems.value, [taskId]: response.items };
    expandedError.value = { ...expandedError.value, [taskId]: null };
  } catch (error) {
    expandedError.value = {
      ...expandedError.value,
      [taskId]: summarizeApiError(error, ITEM_ERROR_TOOLTIP_MAX),
    };
  } finally {
    expandingTaskId.value = null;
  }
}

async function toggleExpand(row: { original: TaskResponse; id: string }) {
  const taskId = row.original.task_id;
  const next = !expandedRows.value[row.id];
  expandedRows.value = { ...expandedRows.value, [row.id]: next };
  if (!next || expandedItems.value[taskId] !== undefined) return;
  await loadTaskItems(taskId);
}

async function retryLoadItems(taskId: string) {
  if (expandingTaskId.value === taskId) return;
  await loadTaskItems(taskId);
}

function clampText(value: string | null | undefined, max: number): string | null {
  if (!value) return null;
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

function itemErrorTooltip(message: string | null | undefined): string | null {
  return clampText(message, ITEM_ERROR_TOOLTIP_MAX);
}

// Refresh expanded item lists for tasks that are still active whenever the
// visible task rows change.
watch(
  () => props.items.map((task) => task.updated_at).join(","),
  () => {
    const visibleExpanded = Object.keys(expandedRows.value).filter(
      (rowId) => expandedRows.value[rowId],
    );
    const taskIds = visibleExpanded.map((rowId) => props.items[Number(rowId)]?.task_id).filter(Boolean);
    for (const taskId of taskIds) {
      const task = props.items.find((candidate) => candidate.task_id === taskId);
      if (!task || !["queued", "running", "waiting"].includes(task.status)) continue;
      void apiClient
        .getTaskItems(taskId, { limit: 100 })
        .then((response) => {
          expandedItems.value = { ...expandedItems.value, [taskId]: response.items };
        })
        .catch(() => undefined);
    }
  },
);

const columns = computed<TableColumn<TaskResponse>[]>(() => [
  { id: "expand", enableHiding: false },
  { accessorKey: "task_id", header: t("processingQueue.task") },
  { accessorKey: "kind", header: t("processingQueue.type"), enableSorting: true },
  { accessorKey: "group_path", header: t("processingQueue.group"), enableSorting: true },
  { accessorKey: "status", header: t("processingQueue.status"), enableSorting: true },
  { accessorKey: "stage", header: t("processingQueue.stage"), enableSorting: true },
  { id: "waiting", header: t("processingQueue.waiting") },
  { id: "progress", header: t("processingQueue.progress") },
  { id: "error", header: t("processingQueue.error") },
  { accessorKey: "updated_at", header: t("processingQueue.updatedAt"), enableSorting: true },
  { id: "actions", header: t("processingQueue.actions") },
]);

function taskStatusLabel(status: TaskStatus) { return t(`processingQueue.statuses.${status}`); }
function taskKindLabel(kind: TaskResponse["kind"]) { return t(`processingQueue.kinds.${kind}`); }
function stageLabel(stage: string | null) { return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage"); }
function waitingLabel(reason: string | null, dependency: string | null) {
  if (!reason) return "--";
  const label = t(`processingQueue.waitingReasons.${reason}`);
  return dependency ? `${label}: ${libraryDependencyLabel(t, dependency)}` : label;
}
function statusSeverity(status: TaskStatus): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}
</script>

<template>
  <UTable
    v-model:sorting="sorting"
    class="min-w-0"
    :ui="{ root: 'overflow-visible', base: 'min-w-[88rem]' }"
    data-testid="processing-queue-table"
    v-model:expanded="expandedRows"
    :data="items"
    :columns="columns"
    :loading="loading"
    :sorting-options="{ manualSorting: true }"
  >
    <template #expand-cell="{ row }">
      <UButton
        variant="ghost"
        color="neutral"
        size="sm"
        icon="i-lucide-chevron-right"
        :class="{ 'rotate-90': row.getIsExpanded() }"
        :aria-label="row.getIsExpanded() ? t('processingQueue.collapse') : t('processingQueue.expand')"
        :aria-expanded="row.getIsExpanded()"
        :disabled="expandingTaskId === row.original.task_id"
        @click="toggleExpand(row)"
      />
    </template>
    <template #task_id-cell="{ row }"><span class="block max-w-64 truncate font-mono text-xs" :title="row.original.task_id">{{ row.original.task_id }}</span></template>
    <template #kind-cell="{ row }"><UBadge :label="taskKindLabel(row.original.kind)" color="neutral" variant="subtle" /></template>
    <template #group_path-cell="{ row }"><span class="block max-w-48 truncate" :title="row.original.group_path || undefined">{{ row.original.group_path || "--" }}</span></template>
    <template #status-cell="{ row }"><UBadge :label="taskStatusLabel(row.original.status)" :color="statusSeverity(row.original.status)" variant="subtle" /></template>
    <template #stage-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ stageLabel(row.original.stage) }}</span></template>
    <template #waiting-cell="{ row }"><span class="block max-w-48 truncate text-sm text-muted" :title="waitingLabel(row.original.waiting_reason, row.original.dependency_key)">{{ waitingLabel(row.original.waiting_reason, row.original.dependency_key) }}</span></template>
    <template #progress-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ row.original.progress.succeeded }}/{{ row.original.progress.total }}</span></template>
    <template #error-cell="{ row }"><span class="block max-w-80 truncate text-sm text-muted" :title="itemErrorTooltip(row.original.error_summary) || undefined">{{ row.original.error_summary || "--" }}</span></template>
    <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.updated_at) }}</span></template>
    <template #actions-cell="{ row }">
      <div class="flex items-center gap-1">
        <UButton v-if="isRecoverableTask(row.original)" color="neutral" variant="ghost" size="sm" icon="i-lucide-rotate-ccw" :loading="isActing(row.original)" :label="t(isDoclingRecoveryTask(row.original) ? 'processingQueue.doclingRecovery' : row.original.status === 'cancelled' ? 'processingQueue.resubmit' : 'processingQueue.retry')" :title="row.original.status === 'cancelled' ? t('processingQueue.resubmitHint') : undefined" @click="emit('recover', row.original)" />
        <UButton v-if="['queued', 'running', 'waiting'].includes(row.original.status)" color="error" variant="ghost" size="sm" icon="i-lucide-ban" :loading="isActing(row.original)" :aria-label="t('processingQueue.cancel')" :title="t('processingQueue.cancel')" @click="emit('cancel', row.original)" />
      </div>
    </template>
    <template #expanded="{ row }">
      <TaskItemsExpanded
        :task="row.original"
        :items="expandedItems[row.original.task_id]"
        :error="expandedError[row.original.task_id]"
        :is-loading="expandingTaskId === row.original.task_id"
        :is-admin="isAdmin"
        :is-acting="isActing(row.original)"
        @retry="emit('recover', $event)"
        @recover="emit('recoverItem', $event)"
        @retry-load="retryLoadItems"
      />
    </template>
  </UTable>
</template>
