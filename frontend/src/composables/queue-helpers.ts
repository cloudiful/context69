import type { SortDirection, TaskListView, TaskResponse, TaskSortBy, TaskStatus } from "../services/api";

export const ACTIVE_STATUSES: TaskStatus[] = ["queued", "running", "waiting"];
export const TERMINAL_STATUSES: TaskStatus[] = ["succeeded", "failed", "cancelled"];

// The processing view predicate is `status <> 'succeeded'`, so its status
// filter never offers the completed value; completed and trash have no
// status selector at all.
export const PROCESSING_STATUS_FILTERS: TaskStatus[] = ["queued", "running", "waiting", "failed", "cancelled"];

// Task stage vocabulary (issue 635): `create.sql` seeds the task stage from the
// kind entry stage (download/storage/sync/delete/translation/indexing), while
// collapsed items report processing/finalize. The filter options and both
// stage renderers share this list so locale keys never drift from valid
// values again.
export const QUEUE_STAGE_VALUES = [
  "download",
  "storage",
  "sync",
  "delete",
  "translation",
  "indexing",
  "processing",
  "finalize",
] as const;

export type QueueStage = (typeof QUEUE_STAGE_VALUES)[number];

const QUEUE_STAGE_SET: ReadonlySet<string> = new Set(QUEUE_STAGE_VALUES);

type StageTranslate = (key: string, params?: Record<string, unknown>) => string;

// Unknown or missing stages fall back to the Unknown label so the UI never
// leaks a raw `processingQueue.stages.*` key.
export function queueStageLabel(t: StageTranslate, stage: string | null | undefined): string {
  if (!stage) return t("processingQueue.unknownStage");
  if (QUEUE_STAGE_SET.has(stage)) return t(`processingQueue.stages.${stage}`);
  return t("processingQueue.unknownStage");
}

// Per-view default ordering (issue 629): processing stays stable by creation
// time so progress updates do not move active rows, while completed surfaces
// the most recently updated task first. Trash keeps the backend default
// (creation time). The queue always sends an explicit sort so the SQL order
// is the single source of row order.
export const DEFAULT_QUEUE_SORT_BY_VIEW: Record<TaskListView, { field: TaskSortBy; direction: SortDirection }> = {
  processing: { field: "created_at", direction: "desc" },
  completed: { field: "updated_at", direction: "desc" },
  trash: { field: "created_at", direction: "desc" },
};

export interface RecoverySummary {
  succeeded: number;
  skipped: number;
  failed: number;
}

// A failed task is retryable in place; a cancelled task has to be rerun into
// a fresh task record because its original idempotency key stays bound. Both
// are offered as the single queue-level recovery action.
export function isRecoverableTask(task: TaskResponse): boolean {
  return task.status === "failed" || task.status === "cancelled";
}

export function isTerminalTask(task: TaskResponse): boolean {
  return TERMINAL_STATUSES.includes(task.status);
}

export function summarizeResults(results: PromiseSettledResult<unknown>[], skippedMessagePattern?: RegExp): RecoverySummary {
  const summary: RecoverySummary = { succeeded: 0, skipped: 0, failed: 0 };
  for (const result of results) {
    if (result.status === "fulfilled") {
      summary.succeeded += 1;
    } else if (skippedMessagePattern && result.reason instanceof Error && skippedMessagePattern.test(result.reason.message)) {
      summary.skipped += 1;
    } else {
      summary.failed += 1;
    }
  }
  return summary;
}
