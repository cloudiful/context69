import { apiClient } from "@/services/api";
import type { TaskRef, TaskResponse, TaskStatus } from "@/services/api";

const TERMINAL: ReadonlySet<TaskStatus> = new Set(["succeeded", "failed", "cancelled"]);
const POLL_INTERVAL_MS = 1500;
const MAX_POLLS = 40;

export interface SettleOptions {
  signal?: AbortSignal;
  pollIntervalMs?: number;
  maxPolls?: number;
}

/**
 * Waits for the submitted tasks to reach a terminal state so the library can be
 * refreshed with final statuses. Poll failures are tolerated: a task that cannot
 * be read stops being waited on instead of failing the initiating action.
 */
export async function settleTaskRefs(
  taskRefs: TaskRef[],
  options: SettleOptions = {},
): Promise<TaskResponse[]> {
  const pollIntervalMs = options.pollIntervalMs ?? POLL_INTERVAL_MS;
  const maxPolls = options.maxPolls ?? MAX_POLLS;
  const ids = [...new Set(taskRefs.map((task) => task?.task_id).filter((id): id is string => !!id))];
  if (ids.length === 0) {
    return [];
  }

  const final = new Map<string, TaskResponse>();
  const pending = new Set(ids);
  let polls = 0;

  while (pending.size > 0 && polls < maxPolls) {
    if (options.signal?.aborted) {
      break;
    }
    const results = await Promise.all([...pending].map(async (taskId) => {
      try {
        return await apiClient.getTask(taskId, { signal: options.signal });
      } catch {
        return null;
      }
    }));

    for (const task of results) {
      if (!task) {
        continue;
      }
      final.set(task.task_id, task);
      if (TERMINAL.has(task.status)) {
        pending.delete(task.task_id);
      }
    }

    polls += 1;
    if (pending.size === 0) {
      break;
    }
    if (polls >= maxPolls) {
      break;
    }
    await new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, pollIntervalMs);
      options.signal?.addEventListener("abort", () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
    });
  }

  return ids.flatMap((id) => {
    const task = final.get(id);
    return task ? [task] : [];
  });
}

export function hasFailedTask(results: TaskResponse[]): boolean {
  return results.some((task) => task.status === "failed");
}
