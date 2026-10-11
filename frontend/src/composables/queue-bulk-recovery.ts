import type { TaskResponse } from "../services/api";
import { summarizeResults, type RecoverySummary } from "./queue-helpers";

export type QueueRecoveryAction = "retry" | "rerun";

export interface QueueRecoveryTarget {
  readonly taskId: string;
  readonly action: QueueRecoveryAction;
}

export interface QueueRecoveryOutcome {
  readonly target: QueueRecoveryTarget;
  readonly result: PromiseSettledResult<void>;
}

export interface QueueRecoverySummary {
  summary: RecoverySummary;
  resumedCount: number;
}

const RECOVERY_CONCURRENCY = 4;

export function captureQueueRecoveryTargets(tasks: readonly TaskResponse[]): readonly QueueRecoveryTarget[] {
  return Object.freeze(tasks.map((task) => Object.freeze({
    taskId: task.task_id,
    action: task.status === "cancelled" ? "rerun" : "retry",
  })));
}

export async function runQueueBulkRecovery(
  targets: readonly QueueRecoveryTarget[],
  submit: (taskId: string, action: QueueRecoveryAction) => Promise<unknown>,
): Promise<QueueRecoveryOutcome[]> {
  const outcomes = new Array<QueueRecoveryOutcome>(targets.length);
  let nextIndex = 0;

  async function runWorker(): Promise<void> {
    while (true) {
      const index = nextIndex++;
      const target = targets[index];
      if (!target) return;
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

export function summarizeQueueRecovery(outcomes: readonly QueueRecoveryOutcome[]): QueueRecoverySummary {
  const results = outcomes.map(({ result }) => result);
  const summary = summarizeResults(results, (reason, index) =>
    outcomes[index]?.target.action === "retry"
    && reason instanceof Error
    && /no retryable/i.test(reason.message));
  const resumedCount = outcomes.filter(({ target, result }) =>
    target.action === "rerun" && result.status === "fulfilled").length;
  return { summary, resumedCount };
}
