<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import TaskItemsExpanded from "../TaskItemsExpanded.vue";
import type { SortDirection, TaskKind, TaskListView, TaskResponse, TaskSortBy, TaskStatus } from "../../services/api";
import { formatTimestamp } from "../../utils/format";
import { libraryDependencyLabel } from "../../utils/library-status";
import QueueHeaderFilter from "./QueueHeaderFilter.vue";
import { createQueueFilterSelects, useQueueFilterOptions, useQueueTableColumns } from "./queue-table-filters";

const props = defineProps<{
  items: TaskResponse[];
  loading: boolean;
  view: TaskListView;
  sort: { field: TaskSortBy; direction: SortDirection } | null;
  statusFilter: TaskStatus | null;
  kindFilter: TaskKind | null;
  stageFilter: string | null;
  waitingReasonFilter: string | null;
  dependencyKeyFilter: string | null;
  isActing: (task: TaskResponse) => boolean;
  isRecoverableTask: (task: TaskResponse) => boolean;
}>();

const emit = defineEmits<{
  recover: [task: TaskResponse];
  cancel: [task: TaskResponse];
  trash: [task: TaskResponse];
  restore: [task: TaskResponse];
  delete: [task: TaskResponse];
  sort: [value: { field: TaskSortBy; direction: "asc" | "desc" } | null];
  "update:statusFilter": [value: TaskStatus | null];
  "update:kindFilter": [value: TaskKind | null];
  "update:stageFilter": [value: string | null];
  "update:waitingReasonFilter": [value: string | null];
  "update:dependencyKeyFilter": [value: string | null];
}>();

const { t } = useI18n();

// The queue renders the 6 backend task states 1:1 so badges match the
// status filter vocabulary (waiting renders 等待中, cancelled renders 已取消).
// Stage values collapsed to processing/finalize in issue 529, so the task
// stage renders 1:1 as well.
const ACTIVE_TASK_STATUSES: TaskStatus[] = ["queued", "running", "waiting"];

const SORTABLE_FIELDS: TaskSortBy[] = ["kind", "group_path", "status", "stage", "created_at", "updated_at"];
const TIME_SORT_FIELDS: TaskSortBy[] = ["created_at", "updated_at"];

// Expanded-row item paging/filtering lives inside TaskItemsExpanded (issue 413
// Phase 3): the table only tracks which rows are open. Item fetching follows
// `next_cursor` per task with the Phase 1 `status` filter, so a 2781-item task
// pages without bloating this table.
const expandedRows = ref<Record<string, boolean>>({});
const sorting = ref<{ id: string; desc: boolean }[]>(
  props.sort ? [{ id: props.sort.field, desc: props.sort.direction === "desc" }] : [],
);

watch(sorting, (value) => {
  const next = value[0];
  if (!next) {
    emit("sort", null);
    return;
  }
  if (!SORTABLE_FIELDS.includes(next.id as TaskSortBy)) return;
  emit("sort", { field: next.id as TaskSortBy, direction: next.desc ? "desc" : "asc" });
});

// The parent owns the sort state (including per-view defaults), so a tab
// switch or a reload re-syncs the visible indicator instead of leaving a
// stale one behind. The equality guard avoids echoing the state back.
watch(() => props.sort, (next) => {
  const state = next ? [{ id: next.field, desc: next.direction === "desc" }] : [];
  if (JSON.stringify(state) !== JSON.stringify(sorting.value)) {
    sorting.value = state;
  }
});

// Header sort buttons cycle new field -> preferred direction -> flipped
// direction -> view default (via null). Time fields prefer newest-first,
// matching the per-view defaults; every state stays server-side.
function cycleSort(field: TaskSortBy) {
  const current = props.sort;
  const preferred = TIME_SORT_FIELDS.includes(field) ? "desc" : "asc";
  if (current?.field !== field) {
    emit("sort", { field, direction: preferred });
    return;
  }
  if (current.direction === preferred) {
    emit("sort", { field, direction: preferred === "desc" ? "asc" : "desc" });
    return;
  }
  emit("sort", null);
}

function sortState(field: TaskSortBy): "asc" | "desc" | "none" {
  if (props.sort?.field !== field) return "none";
  return props.sort.direction;
}

function sortIcon(field: TaskSortBy): string {
  const state = sortState(field);
  if (state === "asc") return "i-lucide-arrow-up";
  if (state === "desc") return "i-lucide-arrow-down";
  return "i-lucide-chevrons-up-down";
}

function sortAriaLabel(field: TaskSortBy, columnLabel: string): string {
  const state = sortState(field);
  const stateLabel = state === "asc"
    ? t("processingQueue.sortedAscending")
    : state === "desc" ? t("processingQueue.sortedDescending") : t("processingQueue.notSorted");
  return `${t("processingQueue.sortBy", { column: columnLabel })}, ${stateLabel}`;
}

const trashView = computed(() => props.view === "trash");
const completedView = computed(() => props.view === "completed");

// Completed hides the stage/waiting/progress columns (terminal view) and
// shows the library dependency as its own column so its filter stays on a
// visible, understandable header.
const { columns, updatedHeader } = useQueueTableColumns(t, completedView);

// Column-specific filters reuse the queue filter state through header
// popovers; completed hides the status/stage/waiting-reason selectors because
// its view is already terminal, while type and dependency filtering stay.
const { statusOptions, kindOptions, stageOptions, waitingReasonOptions, dependencyOptions } = useQueueFilterOptions(t);
const { selectStatusFilter, selectKindFilter, selectStageFilter, selectWaitingReasonFilter, selectDependencyKeyFilter } = createQueueFilterSelects(emit);

function toggleExpand(row: { original: TaskResponse; id: string }) {
  const next = !expandedRows.value[row.id];
  expandedRows.value = { ...expandedRows.value, [row.id]: next };
}

function taskStatusLabel(status: TaskStatus) {
  return t(`processingQueue.statuses.${status}`);
}
function taskKindLabel(kind: TaskResponse["kind"]) { return t(`processingQueue.kinds.${kind}`); }
function stageLabel(stage: string | null) {
  return stage ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage");
}
function waitingLabel(reason: string | null, dependency: string | null) {
  if (!reason) return "--";
  const label = t(`processingQueue.waitingReasons.${reason}`);
  return dependency ? `${label}: ${libraryDependencyLabel(t, dependency)}` : label;
}
function dependencyLabel(dependency: string | null) {
  return dependency ? libraryDependencyLabel(t, dependency) : "--";
}
function statusSeverity(status: TaskStatus): "success" | "error" | "warning" | "neutral" | "primary" {
  if (status === "succeeded") return "success";
  if (status === "failed") return "error";
  if (status === "queued") return "warning";
  if (status === "waiting") return "warning";
  if (status === "running") return "primary";
  return "neutral";
}

// One labeled primary action per display state: active (queued/running/waiting)
// cancels, failed retries, cancelled resumes (rerun), succeeded trashes. The
// trash icon on failed/cancelled rows is a secondary history action, not a
// primary.
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
    <template #kind-header>
      <div class="flex items-center gap-0.5">
        <UButton variant="ghost" color="neutral" size="xs" :label="t('processingQueue.type')" :icon="sortIcon('kind')" :aria-label="sortAriaLabel('kind', t('processingQueue.type'))" :data-sort="sortState('kind')" data-testid="queue-sort-kind" @click="cycleSort('kind')" />
        <QueueHeaderFilter :label="t('processingQueue.kindFilter')" :options="kindOptions" :selected="props.kindFilter" @select="selectKindFilter" />
      </div>
    </template>
    <template #group_path-header>
      <div class="flex items-center gap-0.5">
        <UButton variant="ghost" color="neutral" size="xs" :label="t('processingQueue.group')" :icon="sortIcon('group_path')" :aria-label="sortAriaLabel('group_path', t('processingQueue.group'))" :data-sort="sortState('group_path')" data-testid="queue-sort-group_path" @click="cycleSort('group_path')" />
      </div>
    </template>
    <template #status-header>
      <div class="flex items-center gap-0.5">
        <UButton variant="ghost" color="neutral" size="xs" :label="t('processingQueue.status')" :icon="sortIcon('status')" :aria-label="sortAriaLabel('status', t('processingQueue.status'))" :data-sort="sortState('status')" data-testid="queue-sort-status" @click="cycleSort('status')" />
        <QueueHeaderFilter v-if="props.view === 'processing'" :label="t('processingQueue.statusFilter')" :options="statusOptions" :selected="props.statusFilter" @select="selectStatusFilter" />
      </div>
    </template>
    <template #stage-header>
      <div class="flex items-center gap-0.5">
        <UButton variant="ghost" color="neutral" size="xs" :label="t('processingQueue.stage')" :icon="sortIcon('stage')" :aria-label="sortAriaLabel('stage', t('processingQueue.stage'))" :data-sort="sortState('stage')" data-testid="queue-sort-stage" @click="cycleSort('stage')" />
        <QueueHeaderFilter v-if="!completedView" :label="t('processingQueue.stageFilter')" :options="stageOptions" :selected="props.stageFilter" @select="selectStageFilter" />
      </div>
    </template>
    <template #dependency-header>
      <div class="flex items-center gap-0.5">
        <span class="px-1 text-sm font-medium">{{ t("processingQueue.dependencyFilter") }}</span>
        <QueueHeaderFilter :label="t('processingQueue.dependencyFilter')" :options="dependencyOptions" :selected="props.dependencyKeyFilter" @select="selectDependencyKeyFilter" />
      </div>
    </template>
    <template #waiting-header>
      <div class="flex items-center gap-0.5">
        <span class="px-1 text-sm font-medium">{{ t("processingQueue.waiting") }}</span>
        <QueueHeaderFilter v-if="!completedView" :label="t('processingQueue.waitingReasonFilter')" :options="waitingReasonOptions" :selected="props.waitingReasonFilter" @select="selectWaitingReasonFilter" />
        <QueueHeaderFilter :label="t('processingQueue.dependencyFilter')" :options="dependencyOptions" :selected="props.dependencyKeyFilter" @select="selectDependencyKeyFilter" />
      </div>
    </template>
    <template #created_at-header>
      <div class="flex items-center gap-0.5">
        <UButton variant="ghost" color="neutral" size="xs" :label="t('processingQueue.createdAt')" :icon="sortIcon('created_at')" :aria-label="sortAriaLabel('created_at', t('processingQueue.createdAt'))" :data-sort="sortState('created_at')" data-testid="queue-sort-created_at" @click="cycleSort('created_at')" />
      </div>
    </template>
    <template #updated_at-header>
      <div class="flex items-center gap-0.5">
        <UButton variant="ghost" color="neutral" size="xs" :label="updatedHeader" :icon="sortIcon('updated_at')" :aria-label="sortAriaLabel('updated_at', updatedHeader)" :data-sort="sortState('updated_at')" data-testid="queue-sort-updated_at" @click="cycleSort('updated_at')" />
      </div>
    </template>
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
    <template #dependency-cell="{ row }"><span class="block max-w-48 truncate text-sm text-muted" :title="row.original.dependency_key || undefined">{{ dependencyLabel(row.original.dependency_key) }}</span></template>
    <template #stage-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ stageLabel(row.original.stage) }}</span></template>
    <template #waiting-cell="{ row }"><span class="block max-w-48 truncate text-sm text-muted" :title="waitingLabel(row.original.waiting_reason, row.original.dependency_key)">{{ waitingLabel(row.original.waiting_reason, row.original.dependency_key) }}</span></template>
    <template #progress-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ row.original.progress.succeeded }}/{{ row.original.progress.total }}</span></template>
    <template #error-cell="{ row }"><span class="block max-w-80 truncate text-sm text-muted" :title="row.original.error_summary || undefined">{{ row.original.error_summary || "--" }}</span></template>
    <template #created_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.created_at) }}</span></template>
    <template #updated_at-cell="{ row }"><span class="whitespace-nowrap text-sm text-muted">{{ formatTimestamp(row.original.updated_at) }}</span></template>
    <template #actions-cell="{ row }">
      <div v-if="trashView" class="flex items-center gap-1">
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
        :is-acting="props.isActing(row.original)"
        @retry="emit('recover', $event)"
      />
    </template>
  </UTable>
</template>
