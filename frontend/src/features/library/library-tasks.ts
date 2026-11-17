import { apiClient } from "@/services/api";
import type { TaskRef, TaskResponse, TaskStatus } from "@/services/api";

const TERMINAL: ReadonlySet<TaskStatus> = new Set(["succeeded", "failed", "cancelled"]);
const POLL_INTERVAL_MS = 1500;
const MAX_POLLS = 40;

export interface SettleOptions {
  signal?: AbortSignal;
  pollIntervalMs?: number;
  maxPolls?: number;
  /**
   * Keep observing running/waiting tasks until every one is terminal. The
   * existing bounded callers leave this off; a long reprocess uses it so a
   * failure after the current one-minute window is still reported.
   */
  untilTerminal?: boolean;
  /** Consecutive unreadable polls before an until-terminal task is reported. */
  maxUnreachablePolls?: number;
  /** Called once per batch of tasks that stayed unreadable. */
  onUnreachable?: (taskIds: string[]) => void;
  /** Called with the tasks that reached a terminal state in one poll. */
  onTerminal?: (tasks: TaskResponse[]) => void;
}

function waitForPoll(intervalMs: number, signal?: AbortSignal): Promise<void> {
  return new Promise<void>((resolve) => {
    const onAbort = () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      resolve();
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, intervalMs);
    if (signal?.aborted) {
      onAbort();
      return;
    }
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

/**
 * Waits for the submitted tasks to reach a terminal state so the library can be
 * refreshed with final statuses. Poll failures are tolerated: a task that cannot
 * be read stops being waited on instead of failing the initiating action. In
 * until-terminal mode a task is reported through `onUnreachable` only after
 * several consecutive failed reads, so network loss is never mistaken for
 * completion, and the observation still stops on abort.
 */
export async function settleTaskRefs(
  taskRefs: TaskRef[],
  options: SettleOptions = {},
): Promise<TaskResponse[]> {
  const pollIntervalMs = options.pollIntervalMs ?? POLL_INTERVAL_MS;
  const maxPolls = options.maxPolls ?? MAX_POLLS;
  const untilTerminal = options.untilTerminal ?? false;
  const maxUnreachablePolls = options.maxUnreachablePolls ?? 5;
  const ids = [...new Set(taskRefs.map((task) => task?.task_id).filter((id): id is string => !!id))];
  if (ids.length === 0) {
    return [];
  }

  const final = new Map<string, TaskResponse>();
  const pending = new Set(ids);
  const unreachable = new Map<string, number>();
  let polls = 0;

  while (pending.size > 0 && (untilTerminal || polls < maxPolls)) {
    if (options.signal?.aborted) {
      break;
    }
    const pendingIds = [...pending];
    const results = await Promise.all(pendingIds.map(async (taskId) => {
      try {
        return await apiClient.getTask(taskId, { signal: options.signal });
      } catch {
        return null;
      }
    }));

    const unreachableIds: string[] = [];
    const terminalTasks: TaskResponse[] = [];
    for (const [index, taskId] of pendingIds.entries()) {
      const task = results[index];
      if (!task) {
        if (!untilTerminal) {
          continue;
        }
        const failures = (unreachable.get(taskId) ?? 0) + 1;
        unreachable.set(taskId, failures);
        if (failures >= maxUnreachablePolls) {
          pending.delete(taskId);
          unreachableIds.push(taskId);
        }
        continue;
      }
      unreachable.delete(taskId);
      final.set(task.task_id, task);
      if (TERMINAL.has(task.status)) {
        pending.delete(task.task_id);
        terminalTasks.push(task);
      }
    }
    if (terminalTasks.length > 0 && !options.signal?.aborted) {
      options.onTerminal?.(terminalTasks);
    }
    if (unreachableIds.length > 0 && !options.signal?.aborted) {
      options.onUnreachable?.(unreachableIds);
    }

    polls += 1;
    if (pending.size === 0) {
      break;
    }
    if (!untilTerminal && polls >= maxPolls) {
      break;
    }
    await waitForPoll(pollIntervalMs, options.signal);
  }

  return ids.flatMap((id) => {
    const task = final.get(id);
    return task ? [task] : [];
  });
}

export function hasFailedTask(results: TaskResponse[]): boolean {
  return results.some((task) => task.status === "failed");
}

/**
 * The Processing Queue reads a focused task from `?task=<uuid>`. Library
 * feedback reuses that deep link so a queued or failed reprocessing task can
 * be opened from the toast that reports it.
 */
export function processingQueueTaskPath(taskId: string): string {
  return `/processing-queue?task=${encodeURIComponent(taskId)}`;
}
