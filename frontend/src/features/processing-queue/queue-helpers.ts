import type {
  SortDirection,
  TaskItemResponse,
  TaskKind,
  TaskListView,
  TaskResponse,
  TaskSortBy,
  TaskStatus,
} from "@/services/api";

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

export interface RecoverySummary {
  succeeded: number;
  skipped: number;
  failed: number;
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

export function summarizeResults(
  results: PromiseSettledResult<unknown>[],
  isSkipped?: (reason: unknown, index: number) => boolean,
): RecoverySummary {
  const summary: RecoverySummary = { succeeded: 0, skipped: 0, failed: 0 };
  results.forEach((result, index) => {
    if (result.status === "fulfilled") {
      summary.succeeded += 1;
    } else if (isSkipped?.(result.reason, index)) {
      summary.skipped += 1;
    } else {
      summary.failed += 1;
    }
  });
  return summary;
}

export type QueueRecoveryAction = "retry" | "rerun";

export interface QueueRecoveryTarget {
  readonly taskId: string;
  readonly action: QueueRecoveryAction;
}

export interface QueueRecoveryOutcome {
  readonly target: QueueRecoveryTarget;
  readonly result: PromiseSettledResult<void>;
}

/** The recovery action each captured row needs, decided once at capture time. */
export function captureQueueRecoveryTargets(
  tasks: readonly TaskResponse[],
): readonly QueueRecoveryTarget[] {
  const seen = new Set<string>();
  const targets: QueueRecoveryTarget[] = [];
  for (const task of tasks) {
    if (seen.has(task.task_id)) {
      continue;
    }
    seen.add(task.task_id);
    targets.push(Object.freeze({
      taskId: task.task_id,
      action: task.status === "cancelled" ? "rerun" : "retry",
    }));
  }
  return Object.freeze(targets);
}

const RECOVERY_CONCURRENCY = 4;

/**
 * Bounded recovery: never more than four in-flight submissions, ordered
 * outcomes, and every rejection captured so the summary stays truthful.
 */
export async function runQueueBulkRecovery(
  targets: readonly QueueRecoveryTarget[],
  submit: (taskId: string, action: QueueRecoveryAction) => Promise<unknown>,
): Promise<QueueRecoveryOutcome[]> {
  const outcomes = new Array<QueueRecoveryOutcome>(targets.length);
  let nextIndex = 0;

  async function runWorker(): Promise<void> {
    for (;;) {
      const index = nextIndex;
      nextIndex += 1;
      const target = targets[index];
      if (!target) {
        return;
      }
      try {
        await submit(target.taskId, target.action);
        outcomes[index] = { target, result: { status: "fulfilled", value: undefined } };
      } catch (reason) {
        outcomes[index] = { target, result: { status: "rejected", reason } };
      }
    }
  }

  const workerCount = Math.min(RECOVERY_CONCURRENCY, targets.length);
  await Promise.all(Array.from({ length: workerCount }, () => runWorker()));
  return outcomes;
}

export function summarizeQueueRecovery(outcomes: readonly QueueRecoveryOutcome[]): {
  summary: RecoverySummary;
  resumedCount: number;
} {
  const results = outcomes.map(({ result }) => result);
  const summary = summarizeResults(results, (reason, index) => {
    const target = outcomes[index]?.target;
    return target?.action === "retry"
      && reason instanceof Error
      && /no retryable/i.test(reason.message);
  });
  const resumedCount = outcomes.filter(({ target, result }) => (
    target.action === "rerun" && result.status === "fulfilled"
  )).length;
  return { summary, resumedCount };
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
