import type {
  SortDirection,
  TaskBatchRetryResponse,
  TaskItemResponse,
  TaskKind,
  TaskListView,
  TaskResponse,
  TaskSortBy,
  TaskStatus,
} from "@/services/api";
import type { components } from "@/generated/openapi";

type Schemas = components["schemas"];

/** The ordered per-task outcomes of one bounded batch source rerun. */
export type TaskBatchRerunResponse = Schemas["TaskBatchRerunResponse"];

export const ACTIVE_STATUSES: TaskStatus[] = ["queued", "running", "waiting"];
export const TERMINAL_STATUSES: TaskStatus[] = ["succeeded", "failed", "cancelled"];

/**
 * The processing view predicate is `status <> 'succeeded'`, so its status
 * selector never offers the completed value; completed and trash expose no
 * status selector at all.
 */
export const PROCESSING_STATUS_FILTERS: TaskStatus[] = [
  "queued",
  "running",
  "waiting",
  "failed",
  "cancelled",
];

/** Persisted task stage vocabulary shared by the filter and the stage cell. */
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

/** Waiting-reason vocabulary shared by the filter and the waiting cell. */
export const QUEUE_WAITING_REASON_VALUES = ["dependency", "backoff", "docling"] as const;

export const QUEUE_DEPENDENCY_VALUES = ["s3", "docling", "embedding", "qdrant"] as const;

const LEGACY_DEPENDENCY_ALIASES: Readonly<Record<string, string>> = {
  embedding_vector: "embedding",
};

/**
 * Resolves a dependency key to its localized label. Unknown keys stay visible
 * as-is instead of disappearing behind an unrecognized filter value.
 */
export function libraryDependencyLabel(t: Translate, dependencyKey: string | null): string {
  if (!dependencyKey) {
    return "--";
  }
  const canonical = LEGACY_DEPENDENCY_ALIASES[dependencyKey] ?? dependencyKey;
  return (QUEUE_DEPENDENCY_VALUES as readonly string[]).includes(canonical)
    ? t(`processingQueue.dependencies.${canonical}`)
    : dependencyKey;
}

export const TASK_KIND_VALUES: TaskKind[] = [
  "source_sync",
  "text_batch",
  "file_batch",
  "url_batch",
  "delete_batch",
  "translation",
  "vector_rebuild",
  "git_index",
];

const QUEUE_STAGE_SET: ReadonlySet<string> = new Set(QUEUE_STAGE_VALUES);

type Translate = (key: string, params?: Record<string, unknown>) => string;

/** Unknown or missing stages render the Unknown label, never a raw key. */
export function queueStageLabel(t: Translate, stage: string | null | undefined): string {
  if (!stage) {
    return t("processingQueue.unknownStage");
  }
  return QUEUE_STAGE_SET.has(stage) ? t(`processingQueue.stages.${stage}`) : t("processingQueue.unknownStage");
}

/**
 * Per-view default ordering: processing stays stable by creation time so
 * progress updates do not move active rows, completed surfaces the most
 * recently updated task first, trash keeps creation time.
 */
export const DEFAULT_QUEUE_SORT_BY_VIEW: Record<
  TaskListView,
  { field: TaskSortBy; direction: SortDirection }
> = {
  processing: { field: "created_at", direction: "desc" },
  completed: { field: "updated_at", direction: "desc" },
  trash: { field: "created_at", direction: "desc" },
};

/**
 * A task is skipped, not failed, when the server refuses it because it is not
 * in an eligible state any more: a status race, a missing task or a task the
 * caller may read but not manage. The distinction is reported, never hidden.
 */
export class QueueTaskSkippedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "QueueTaskSkippedError";
  }
}

/**
 * A bulk action that ran only part of its way. The cancel-then-trash flow is
 * explicitly not atomic: a task that was cancelled but could not be moved to
 * the trash stays cancelled and is reported as a partial failure.
 */
export class QueuePartialFailureError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "QueuePartialFailureError";
  }
}

/** The bulk actions the queue offers on a selection. */
export type QueueBulkAction = "retry" | "rerun" | "cancel" | "trash" | "restore" | "delete";

/**
 * The task kinds whose succeeded items can be rerun from their current
 * source. The server stays the authority: it decides per task and reports a
 * skipped id with a reason, so this list only keeps the queue from offering
 * the action where it can never apply.
 */
export const SOURCE_RERUN_KINDS: readonly TaskKind[] = ["file_batch", "url_batch", "text_batch"];

/**
 * Whether one action applies to a task that had this status when it was
 * selected. The single retry action covers both recoverable states: the batch
 * endpoint retries a failed task's items and resumes a cancelled task's items in
 * place, never forking either. The rerun action covers the succeeded tasks
 * that own a rerunnable source. Both target only the selected tasks that could
 * take them; the server re-checks the live state per id, so a task that
 * changed in the meantime is reported as skipped instead of being forced.
 */
export function isQueueBulkActionApplicable(
  action: QueueBulkAction,
  status: TaskStatus,
): boolean {
  switch (action) {
    case "retry":
      return status === "failed" || status === "cancelled";
    case "rerun":
      return status === "succeeded";
    case "cancel":
      return isActiveStatus(status);
    case "trash":
      return isActiveStatus(status) || isTerminalStatus(status);
    case "restore":
    case "delete":
      return true;
    default:
      return false;
  }
}

export interface QueueBulkTarget {
  readonly taskId: string;
  readonly action: QueueBulkAction;
  /** The status the task had when it was selected. */
  readonly status: TaskStatus;
}

export interface QueueBulkOutcome {
  readonly taskId: string;
  readonly status: "succeeded" | "skipped" | "failed";
  /** True when the task was cancelled but could not be moved to the trash. */
  readonly partial: boolean;
  readonly reason?: string;
}

export interface QueueBulkSummary {
  succeeded: number;
  skipped: number;
  failed: number;
  partial: number;
}

/** The actions a selection can take, decided from the selected tasks' status. */
export function isActiveStatus(status: TaskStatus): boolean {
  return ACTIVE_STATUSES.includes(status);
}

export function isTerminalStatus(status: TaskStatus): boolean {
  return TERMINAL_STATUSES.includes(status);
}

/**
 * The bulk actions that apply to a selection.
 *
 * The actions are contextual: a selection of failed tasks offers a retry and
 * nothing else, a selection of active tasks offers a cancellation, a
 * selection of succeeded tasks offers the source rerun, and the trash view
 * offers the restore and the permanent deletion. An action is offered when at
 * least one selected task could take it.
 */
export function availableQueueBulkActions(
  statuses: readonly TaskStatus[],
  view: TaskListView,
): QueueBulkAction[] {
  if (view === "trash") {
    return statuses.length > 0 ? ["restore", "delete"] : [];
  }
  const actions: QueueBulkAction[] = [];
  if (statuses.some((status) => status === "failed" || status === "cancelled")) {
    actions.push("retry");
  }
  if (statuses.some((status) => status === "succeeded")) {
    actions.push("rerun");
  }
  if (statuses.some((status) => isActiveStatus(status))) {
    actions.push("cancel");
  }
  if (statuses.length > 0) {
    actions.push("trash");
  }
  return actions;
}

/** Every selected task becomes one target, in the order the tasks were selected. */
export function captureQueueBulkTargets(
  ids: readonly string[],
  statuses: Readonly<Record<string, TaskStatus>>,
  action: QueueBulkAction,
): readonly QueueBulkTarget[] {
  const targets: QueueBulkTarget[] = [];
  const seen = new Set<string>();
  for (const taskId of ids) {
    if (seen.has(taskId)) {
      continue;
    }
    seen.add(taskId);
    const status = statuses[taskId];
    if (!status) {
      continue;
    }
    targets.push(Object.freeze({ taskId, action, status }));
  }
  return Object.freeze(targets);
}

const BULK_CONCURRENCY = 4;

/**
 * Bounded bulk execution: never more than four task workflows in flight, the
 * outcomes keep the order of the targets, and every rejection is captured so
 * the summary stays truthful.
 */
export async function runQueueBulkTasks(
  targets: readonly QueueBulkTarget[],
  submit: (target: QueueBulkTarget) => Promise<unknown>,
  onProgress?: (completed: number, total: number) => void,
): Promise<QueueBulkOutcome[]> {
  const outcomes = new Array<QueueBulkOutcome>(targets.length);
  let nextIndex = 0;
  let completed = 0;

  async function runWorker(): Promise<void> {
    for (;;) {
      const index = nextIndex;
      nextIndex += 1;
      const target = targets[index];
      if (!target) {
        return;
      }
      try {
        await submit(target);
        outcomes[index] = { taskId: target.taskId, status: "succeeded", partial: false };
      } catch (reason) {
        const skipped = reason instanceof QueueTaskSkippedError
          || isEligibilityRefusal(reason);
        outcomes[index] = {
          taskId: target.taskId,
          status: skipped ? "skipped" : "failed",
          partial: reason instanceof QueuePartialFailureError,
          reason: reason instanceof Error ? reason.message : String(reason),
        };
      }
      completed += 1;
      onProgress?.(completed, targets.length);
    }
  }

  const workerCount = Math.min(BULK_CONCURRENCY, targets.length);
  await Promise.all(Array.from({ length: workerCount }, () => runWorker()));
  return outcomes;
}

/**
 * A refusal that means "this task is not eligible any more": the task changed
 * state, disappeared, or the caller may read it but not manage it. Any other
 * failure is a real failure and is reported as one.
 */
function isEligibilityRefusal(reason: unknown): boolean {
  const status = (reason as { status?: unknown } | null)?.status;
  if (typeof status !== "number") {
    return false;
  }
  return status === 400 || status === 403 || status === 404 || status === 409;
}

/** One batch API call accepts at most this many task ids. */
export const QUEUE_BATCH_RETRY_LIMIT = 100;

/** The queue keeps at most this many batch API requests in flight. */
export const QUEUE_BATCH_RETRY_CONCURRENCY = 4;

/**
 * The per-id outcome shape both bounded batch endpoints answer with: the
 * ordered id, the outcome, and an optional non-sensitive reason.
 */
interface QueueBatchResult {
  readonly task_id: string;
  readonly outcome: "accepted" | "skipped" | "failed";
  readonly reason?: string | null;
}

/**
 * Recover selected failed and cancelled tasks through the bounded batch API.
 *
 * The captured ids are split into chunks of at most 100 and at most four batch
 * requests run at once. Each response item maps back to the id the caller
 * selected, so the summary keeps the requested order and a skipped/failed id is
 * never reported as recovered. A whole chunk that fails marks only its own ids
 * as failed; the other chunks still complete.
 */
export async function runQueueBatchRetry(
  targets: readonly QueueBulkTarget[],
  submit: (taskIds: readonly string[]) => Promise<TaskBatchRetryResponse>,
  onProgress?: (completed: number, total: number) => void,
): Promise<QueueBulkOutcome[]> {
  return runQueueBatchTasks(targets, submit, onProgress);
}

/**
 * Rerun selected succeeded tasks from their current source through the bounded
 * batch API. The batching, ordering, and per-id mapping are the same as the
 * recovery call; only the endpoint and the outcome vocabulary differ.
 */
export async function runQueueBatchRerun(
  targets: readonly QueueBulkTarget[],
  submit: (taskIds: readonly string[]) => Promise<TaskBatchRerunResponse>,
  onProgress?: (completed: number, total: number) => void,
): Promise<QueueBulkOutcome[]> {
  return runQueueBatchTasks(targets, submit, onProgress);
}

/**
 * The shared bounded batch execution: never more than four batch requests in
 * flight, the outcomes keep the order of the targets, and every rejection is
 * captured so the summary stays truthful.
 */
export async function runQueueBatchTasks(
  targets: readonly QueueBulkTarget[],
  submit: (taskIds: readonly string[]) => Promise<{ results: QueueBatchResult[] }>,
  onProgress?: (completed: number, total: number) => void,
): Promise<QueueBulkOutcome[]> {
  const outcomes = new Array<QueueBulkOutcome>(targets.length);
  const indexByTaskId = new Map<string, number>();
  targets.forEach((target, index) => {
    indexByTaskId.set(target.taskId, index);
  });

  const chunks: Array<{ start: number; ids: string[] }> = [];
  for (let start = 0; start < targets.length; start += QUEUE_BATCH_RETRY_LIMIT) {
    chunks.push({
      start,
      ids: targets
        .slice(start, start + QUEUE_BATCH_RETRY_LIMIT)
        .map((target) => target.taskId),
    });
  }

  let completed = 0;
  let nextChunk = 0;

  async function runWorker(): Promise<void> {
    for (;;) {
      const chunkIndex = nextChunk;
      nextChunk += 1;
      const chunk = chunks[chunkIndex];
      if (!chunk) {
        return;
      }
      try {
        const response = await submit(chunk.ids);
        const byId = new Map(response.results.map((item) => [item.task_id, item]));
        for (const taskId of chunk.ids) {
          const index = indexByTaskId.get(taskId);
          if (index === undefined) {
            continue;
          }
          const result = byId.get(taskId);
          if (!result) {
            outcomes[index] = {
              taskId,
              status: "failed",
              partial: false,
              reason: "the batch response omitted this task",
            };
            continue;
          }
          outcomes[index] = {
            taskId,
            status: result.outcome === "accepted"
              ? "succeeded"
              : result.outcome === "skipped" ? "skipped" : "failed",
            partial: false,
            reason: result.reason ?? undefined,
          };
        }
      } catch (reason) {
        const message = reason instanceof Error ? reason.message : String(reason);
        for (const taskId of chunk.ids) {
          const index = indexByTaskId.get(taskId);
          if (index !== undefined) {
            outcomes[index] = { taskId, status: "failed", partial: false, reason: message };
          }
        }
      }
      completed += chunk.ids.length;
      onProgress?.(completed, targets.length);
    }
  }

  const workerCount = Math.min(QUEUE_BATCH_RETRY_CONCURRENCY, chunks.length);
  await Promise.all(Array.from({ length: workerCount }, () => runWorker()));
  return outcomes;
}

export function summarizeQueueBulkOutcomes(
  outcomes: readonly QueueBulkOutcome[],
): QueueBulkSummary {
  const summary: QueueBulkSummary = { succeeded: 0, skipped: 0, failed: 0, partial: 0 };
  for (const outcome of outcomes) {
    if (outcome.status === "succeeded") {
      summary.succeeded += 1;
      continue;
    }
    if (outcome.status === "skipped") {
      summary.skipped += 1;
      continue;
    }
    summary.failed += 1;
    if (outcome.partial) {
      summary.partial += 1;
    }
  }
  return summary;
}

/**
 * Recovery is decided from the task status and the server's decision: a failed
 * task retries in place, a cancelled task is rerun in place. The per-item
 * `retryable` flag is never consulted here, so a row whose item payload says
 * `retryable: false` still offers the manual action and the server decides.
 */
export function isRecoverableTask(task: TaskResponse): boolean {
  return task.status === "failed" || task.status === "cancelled";
}

/**
 * A succeeded task whose source can be read again. The server decides whether
 * the rerun actually applies: a task that owns no rerunnable source is skipped
 * with a reason instead of failing the whole action.
 */
export function isSourceRerunnableTask(task: TaskResponse): boolean {
  return task.status === "succeeded" && SOURCE_RERUN_KINDS.includes(task.kind);
}

/**
 * The per-task detail of a bulk action: which ids were skipped or failed and
 * why.
 */
export interface QueueBulkOutcomeDetail {
  readonly taskId: string;
  readonly reason: string;
}

export function describeQueueBulkOutcomes(
  outcomes: readonly QueueBulkOutcome[],
): readonly QueueBulkOutcomeDetail[] {
  return outcomes
    .filter((outcome) => outcome.status !== "succeeded" && outcome.reason)
    .map((outcome) => ({ taskId: outcome.taskId, reason: outcome.reason as string }));
}

export function isTerminalTask(task: TaskResponse): boolean {
  return TERMINAL_STATUSES.includes(task.status);
}

export function isActiveTask(task: TaskResponse): boolean {
  return ACTIVE_STATUSES.includes(task.status);
}

export function taskFileName(task: TaskResponse): string | null {
  return task.file_name?.trim() || null;
}

export function taskDocumentTitle(task: TaskResponse): string | null {
  return task.document_title?.trim() || null;
}

/**
 * Deterministic row subject: document/task title, then file name, then the
 * group or source the task belongs to. Identifiers stay out of the collapsed
 * row.
 */
export function taskSubject(task: TaskResponse): string | null {
  return taskDocumentTitle(task)
    ?? taskFileName(task)
    ?? task.group_path?.trim()
    ?? task.source_key?.trim()
    ?? null;
}

// Truthful, stage-local progress rendering rules. The generated progress record
// carries what the worker actually observed; every rule below keeps the UI from
// claiming more than that record supports.
export type TaskItemProgressRecord = NonNullable<TaskItemResponse["progress"]>;

export const PIPELINE_PROGRESS_STAGES = [
  "storage",
  "parsing",
  "docling",
  "chunking",
  "indexing",
  "finalizing",
] as const;

const PROGRESS_STAGE_SET: ReadonlySet<string> = new Set(PIPELINE_PROGRESS_STAGES);

export function pipelineProgressStageLabel(
  t: Translate,
  stage: string | null | undefined,
): string {
  if (!stage || !PROGRESS_STAGE_SET.has(stage)) {
    return t("processingQueue.unknownStage");
  }
  return t(`processingQueue.progressStageLabels.${stage}`);
}

export interface ProgressFraction {
  completed: number;
  total: number;
}

export type ProgressCount = ProgressFraction | null;

function readCount(value: number | null | undefined): number | null {
  if (value == null || !Number.isFinite(value)) {
    return null;
  }
  if (!Number.isInteger(value) || value < 0) {
    return null;
  }
  return value;
}

function readFraction(
  completed: number | null | undefined,
  total: number | null | undefined,
): ProgressCount {
  const done = readCount(completed);
  const whole = readCount(total);
  if (done === null || whole === null || done > whole) {
    return null;
  }
  return { completed: done, total: whole };
}

export interface ItemProgressView {
  stageLabel: string;
  stage: string | null;
  chunks: ProgressCount;
  batches: ProgressCount;
  isComplete: boolean;
  isUnknown: boolean;
  remoteStatus: string | null;
  showsPageProgressUnavailable: boolean;
  lastProgressAt: string | null;
}

export function itemProgressView(
  t: Translate,
  progress: TaskItemProgressRecord | null | undefined,
): ItemProgressView | null {
  if (!progress) {
    return null;
  }
  const stage = typeof progress.stage === "string" ? progress.stage : null;
  const chunks = readFraction(progress.completed_chunks, progress.total_chunks);
  const batches = readFraction(progress.completed_batches, progress.total_batches);
  const lastProgressAt = typeof progress.last_progress_at === "string" ? progress.last_progress_at : null;
  const remoteStatus = typeof progress.remote_status === "string" && progress.remote_status
    ? progress.remote_status
    : null;
  return {
    stageLabel: pipelineProgressStageLabel(t, stage),
    stage,
    chunks,
    batches,
    isComplete: (chunks !== null && chunks.completed === chunks.total)
      || (batches !== null && batches.completed === batches.total && chunks === null),
    isUnknown: chunks === null && batches === null,
    remoteStatus,
    showsPageProgressUnavailable: stage === "docling",
    lastProgressAt,
  };
}

/** Only a known total greater than zero justifies a percentage. */
export function progressPercent(count: ProgressCount): number | null {
  if (!count || count.total <= 0) {
    return null;
  }
  return Math.round((count.completed / count.total) * 100);
}

export function progressCountText(
  t: Translate,
  count: ProgressCount,
  unit: "chunks" | "batches",
): string | null {
  if (!count) {
    return null;
  }
  return t(`processingQueue.itemProgress.${unit}Count`, {
    completed: count.completed,
    total: count.total,
  });
}
