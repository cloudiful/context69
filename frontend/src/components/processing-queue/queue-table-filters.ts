import { computed, type Ref } from "vue";
import type { TableColumn } from "@nuxt/ui";

import type { TaskKind, TaskResponse, TaskStatus } from "../../services/api";
import { PROCESSING_STATUS_FILTERS } from "../../composables/queue-helpers";
import type { QueueHeaderFilterOption } from "./QueueHeaderFilter.vue";

type Translate = (key: string, params?: Record<string, unknown>) => string;

const TASK_KIND_FILTERS: TaskKind[] = ["source_sync", "text_batch", "file_batch", "url_batch", "delete_batch", "translation", "vector_rebuild"];
const STAGE_FILTERS = ["processing", "finalize"];
const WAITING_REASON_FILTERS = ["dependency", "backoff"];
const DEPENDENCY_FILTERS = ["s3", "docling", "embedding", "qdrant"];

// Header filter popovers reuse the queue filter state; the option lists live
// here so the table keeps column/sort ownership in one place.
export function useQueueFilterOptions(t: Translate) {
  const statusOptions = computed<QueueHeaderFilterOption[]>(() => [
    { label: t("processingQueue.allStatuses"), value: null },
    ...PROCESSING_STATUS_FILTERS.map((value) => ({ label: t(`processingQueue.statuses.${value}`), value })),
  ]);
  const kindOptions = computed<QueueHeaderFilterOption[]>(() => [
    { label: t("processingQueue.allKinds"), value: null },
    ...TASK_KIND_FILTERS.map((value) => ({ label: t(`processingQueue.kinds.${value}`), value })),
  ]);
  const stageOptions = computed<QueueHeaderFilterOption[]>(() => [
    { label: t("processingQueue.allStages"), value: null },
    ...STAGE_FILTERS.map((value) => ({ label: t(`processingQueue.stages.${value}`), value })),
  ]);
  const waitingReasonOptions = computed<QueueHeaderFilterOption[]>(() => [
    { label: t("processingQueue.allWaitingReasons"), value: null },
    ...WAITING_REASON_FILTERS.map((value) => ({ label: t(`processingQueue.waitingReasons.${value}`), value })),
  ]);
  const dependencyOptions = computed<QueueHeaderFilterOption[]>(() => [
    { label: t("processingQueue.allDependencies"), value: null },
    ...DEPENDENCY_FILTERS.map((value) => ({ label: t(`processingQueue.dependencies.${value}`), value })),
  ]);
  return { statusOptions, kindOptions, stageOptions, waitingReasonOptions, dependencyOptions };
}

export interface QueueFilterEmit {
  (event: "update:statusFilter", value: TaskStatus | null): void;
  (event: "update:kindFilter", value: TaskKind | null): void;
  (event: "update:stageFilter", value: string | null): void;
  (event: "update:waitingReasonFilter", value: string | null): void;
  (event: "update:dependencyKeyFilter", value: string | null): void;
}

// Filter popovers emit plain strings; the casts restore the queue's filter
// vocabulary the same way the toolbar selects did.
export function createQueueFilterSelects(emit: QueueFilterEmit) {  return {
    selectStatusFilter(value: string | null) { emit("update:statusFilter", value as TaskStatus | null); },
    selectKindFilter(value: string | null) { emit("update:kindFilter", value as TaskKind | null); },
    selectStageFilter(value: string | null) { emit("update:stageFilter", value); },
    selectWaitingReasonFilter(value: string | null) { emit("update:waitingReasonFilter", value); },
    selectDependencyKeyFilter(value: string | null) { emit("update:dependencyKeyFilter", value); },
  };
}

// Completed is a terminal view: stage, waiting, and progress carry no signal
// there, so the table hides those columns and shows the library dependency
// (which stays filterable) as its own column instead of inside waiting.
export function useQueueTableColumns(t: Translate, completed: Ref<boolean>) {
  const updatedHeader = computed(() => completed.value ? t("processingQueue.completedAt") : t("processingQueue.updatedAt"));
  const columns = computed<TableColumn<TaskResponse>[]>(() => {
    const leading: TableColumn<TaskResponse>[] = [
      { id: "expand", enableHiding: false },
      { accessorKey: "task_id", header: t("processingQueue.task") },
      { accessorKey: "kind", header: t("processingQueue.type"), enableSorting: true },
      { accessorKey: "group_path", header: t("processingQueue.group"), enableSorting: true },
      { accessorKey: "status", header: t("processingQueue.status"), enableSorting: true },
    ];
    const middle: TableColumn<TaskResponse>[] = completed.value
      ? [{ id: "dependency", header: t("processingQueue.dependencyFilter") }]
      : [
          { accessorKey: "stage", header: t("processingQueue.stage"), enableSorting: true },
          { id: "waiting", header: t("processingQueue.waiting") },
          { id: "progress", header: t("processingQueue.progress") },
        ];
    return [
      ...leading,
      ...middle,
      { id: "error", header: t("processingQueue.error") },
      { accessorKey: "created_at", header: t("processingQueue.createdAt"), enableSorting: true },
      { accessorKey: "updated_at", header: updatedHeader.value, enableSorting: true },
      { id: "actions", header: t("processingQueue.actions") },
    ];
  });
  return { columns, updatedHeader };
}
