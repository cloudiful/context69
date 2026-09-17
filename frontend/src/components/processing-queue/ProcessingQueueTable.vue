<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { TableColumn } from "@nuxt/ui";
import { useI18n } from "vue-i18n";

import TaskItemsExpanded from "../TaskItemsExpanded.vue";
import type { TaskResponse, TaskSortBy, TaskStatus } from "../../services/api";
import { formatTimestamp } from "../../utils/format";
import { libraryDependencyLabel } from "../../utils/library-status";

const props = defineProps<{
  items: TaskResponse[];
  loading: boolean;
  isAdmin: boolean;
  trashView: boolean;
  isActing: (task: TaskResponse) => boolean;
  isRecoverableTask: (task: TaskResponse) => boolean;
  isDoclingRecoveryTask: (task: TaskResponse) => boolean;
}>();

const emit = defineEmits<{
  recover: [task: TaskResponse];
  recoverItem: [task: TaskResponse];
  cancel: [task: TaskResponse];
  trash: [task: TaskResponse];
  restore: [task: TaskResponse];
  delete: [task: TaskResponse];
  sort: [value: { field: TaskSortBy; direction: "asc" | "desc" } | null];
}>();

const { t } = useI18n();

// Issue 446 P2: the queue converges backend task statuses into 5 display
// states (Thunder-style): queued/waiting collapse into 排队中, cancelled maps
// to 已暂停, the rest map 1:1. Docling's internal stages (docling/docling_poll)
// collapse into a single 转格式中 stage. Backend values still drive filters
// and actions; only the labels converge here.
type DisplayStatus = "queued" | "running" | "paused" | "succeeded" | "failed";
const CONVERTING_STAGES = new Set(["docling", "docling_poll"]);
const ACTIVE_TASK_STATUSES: TaskStatus[] = ["queued", "running", "waiting"];

function displayStatus(status: TaskStatus): DisplayStatus {
  if (status === "waiting") return "queued";
  if (status === "cancelled") return "paused";
  return status;
}

const SORTABLE_FIELDS: TaskSortBy[] = ["kind", "group_path", "status", "stage", "updated_at"];

// Expanded-row item paging/filtering lives inside TaskItemsExpanded (issue 413
// Phase 3): the table only tracks which rows are open. Item fetching follows
// `next_cursor` per task with the Phase 1 `status` filter, so a 2781-item task
// pages without bloating this table.
const expandedRows = ref<Record<string, boolean>>({});
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

function toggleExpand(row: { original: TaskResponse; id: string }) {
  const next = !expandedRows.value[row.id];
  expandedRows.value = { ...expandedRows.value, [row.id]: next };
}

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

function taskStatusLabel(status: TaskStatus) {
  const display = displayStatus(status);
  return t(`processingQueue.statuses.${display}`);
}
function taskKindLabel(kind: TaskResponse["kind"]) { return t(`processingQueue.kinds.${kind}`); }
function stageLabel(stage: string | null) {
  if (stage && CONVERTING_STAGES.has(stage)) return t("processingQueue.stages.converting");
  return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage");
}
function waitingLabel(reason: string | null, dependency: string | null) {
  if (!reason) return "--";
  const label = t(`processingQueue.waitingReasons.${reason}`);
  return dependency ? `${label}: ${libraryDependencyLabel(t, dependency)}` : label;
}
function statusSeverity(status: TaskStatus): "success" | "error" | "warning" | "neutral" | "primary" {
  const display = displayStatus(status);
  if (display === "succeeded") return "success";
  if (display === "failed") return "error";
  if (display === "queued") return "warning";
  if (display === "running") return "primary";
  return "neutral";
}

// One labeled primary action per display state: active (queued/running) pauses,
// failed retries (the unified entry auto-decides retry vs Docling recovery),
// paused resumes (rerun), succeeded trashes. The trash icon on failed/paused
// rows is a secondary history action, not a primary.
function primaryAction(task: TaskResponse): "cancel" | "retry" | "resume" | "trash" | null {
  if (ACTIVE_TASK_STATUSES.includes(task.status)) return "cancel";
  if (task.status === "failed") return "retry";
  if (task.status === "cancelled") return "resume";
  if (task.status === "succeeded") return "trash";
  return null;
}
</script>

<template>
  <UTable
    v-model:sorting="sorting"
    class="min-w-0"
    :ui="{ root: 'overflow-visible', base: 'min-w-[88rem]', th: 'whitespace-nowrap' }"
    data-testid="processing-queue-table"
    v-model:expanded="expandedRows"
    :data="props.items"
    :columns="columns"
    :loading="props.loading"
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
    <template #error-cell="{ row }"><span class="block max-w-80 truncate text-sm text-muted" :title="row.original.error_summary || undefined">{{ row.original.error_summary || "--" }}</span></template>
    <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.updated_at) }}</span></template>
    <template #actions-cell="{ row }">
      <div v-if="props.trashView" class="flex items-center gap-1">
        <UButton color="neutral" variant="ghost" size="sm" icon="i-lucide-undo-2" :loading="props.isActing(row.original)" :aria-label="t('processingQueue.restore')" :title="t('processingQueue.restore')" @click="emit('restore', row.original)" />
        <UButton color="error" variant="ghost" size="sm" icon="i-lucide-trash-2" :loading="props.isActing(row.original)" :aria-label="t('processingQueue.deletePermanently')" :title="t('processingQueue.deletePermanently')" @click="emit('delete', row.original)" />
      </div>
      <div v-else class="flex items-center gap-1">
        <UButton v-if="primaryAction(row.original) === 'retry' && props.isRecoverableTask(row.original)" color="neutral" variant="ghost" size="sm" icon="i-lucide-rotate-ccw" :loading="props.isActing(row.original)" :label="t('processingQueue.retry')" :title="t('processingQueue.retry')" @click="emit('recover', row.original)" />
        <UButton v-if="primaryAction(row.original) === 'resume' && props.isRecoverableTask(row.original)" color="neutral" variant="ghost" size="sm" icon="i-lucide-play" :loading="props.isActing(row.original)" :label="t('processingQueue.continue')" :title="t('processingQueue.resubmitHint')" @click="emit('recover', row.original)" />
        <UButton v-if="primaryAction(row.original) === 'cancel'" color="error" variant="ghost" size="sm" icon="i-lucide-ban" :loading="props.isActing(row.original)" :aria-label="t('processingQueue.cancel')" :title="t('processingQueue.cancel')" @click="emit('cancel', row.original)" />
        <UButton v-if="primaryAction(row.original) === 'trash' || row.original.status === 'failed' || row.original.status === 'cancelled'" color="neutral" variant="ghost" size="sm" icon="i-lucide-trash-2" :loading="props.isActing(row.original)" :aria-label="t('processingQueue.trash')" :title="t('processingQueue.trash')" @click="emit('trash', row.original)" />
      </div>
    </template>
    <template #expanded="{ row }">
      <TaskItemsExpanded
        :task="row.original"
        :is-admin="props.isAdmin"
        :is-acting="props.isActing(row.original)"
        @retry="emit('recover', $event)"
        @recover="emit('recoverItem', $event)"
      />
    </template>
  </UTable>
</template>
