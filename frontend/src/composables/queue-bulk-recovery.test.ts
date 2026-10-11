import { afterEach, describe, expect, it, vi } from "vitest";

import type { TaskResponse } from "../services/api";
import {
  captureQueueRecoveryTargets,
  runQueueBulkRecovery,
  summarizeQueueRecovery,
} from "./queue-bulk-recovery";

function task(taskId: string, status: TaskResponse["status"]): TaskResponse {
  return {
    task_id: taskId,
    kind: "file_batch",
    origin: "manual",
    status,
    group_path: "research",
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 0 },
    failure_stage: "indexing",
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-07-20T00:00:00Z",
    updated_at: "2026-07-20T00:01:00Z",
  };
}

afterEach(() => vi.useRealTimers());

describe("bounded bulk queue recovery", () => {
  it("captures action kinds once, limits work to four, and returns ordered settled outcomes", async () => {
    vi.useFakeTimers();
    const tasks = Array.from({ length: 8 }, (_, index) =>
      task(`task-${index}`, index % 2 === 0 ? "failed" : "cancelled"));
    const targets = captureQueueRecoveryTargets(tasks);
    const calls: Array<{ taskId: string; action: string }> = [];
    let active = 0;
    let maximumActive = 0;
    const submit = vi.fn(async (taskId: string, action: "retry" | "rerun") => {
      calls.push({ taskId, action });
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      await new Promise<void>((resolve, reject) => {
        const ordinal = Number(taskId.slice(-1));
        setTimeout(() => {
          active -= 1;
          if (taskId === "task-2" || taskId === "task-3") {
            reject(new Error("no retryable failed items"));
          } else {
            resolve();
          }
        }, (7 - ordinal) * 10);
      });
    });

    tasks[0]!.status = "cancelled";
    const running = runQueueBulkRecovery(targets, submit);
    await vi.runAllTimersAsync();
    const outcomes = await running;

    expect(maximumActive).toBe(4);
    expect(calls).toHaveLength(8);
    expect(targets.map(({ action }) => action)).toEqual([
      "retry", "rerun", "retry", "rerun", "retry", "rerun", "retry", "rerun",
    ]);
    expect(outcomes.map(({ target }) => target.taskId)).toEqual(tasks.map(({ task_id }) => task_id));
    expect(outcomes.map(({ result }) => result.status)).toEqual([
      "fulfilled", "fulfilled", "rejected", "rejected", "fulfilled", "fulfilled", "fulfilled", "fulfilled",
    ]);
    expect(summarizeQueueRecovery(outcomes)).toEqual({
      summary: { succeeded: 6, skipped: 1, failed: 1 },
      resumedCount: 3,
    });
  });
});
