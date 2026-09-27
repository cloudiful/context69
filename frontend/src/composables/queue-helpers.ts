import type { TaskResponse, TaskStatus } from "../services/api";

export const ACTIVE_STATUSES: TaskStatus[] = ["queued", "running", "waiting"];
export const TERMINAL_STATUSES: TaskStatus[] = ["succeeded", "failed", "cancelled"];

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
