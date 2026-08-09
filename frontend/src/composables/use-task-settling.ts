import { apiClient, type TaskRef, type TaskResponse, type TaskStatus } from "../services/api";

const TERMINAL_STATUSES: ReadonlySet<TaskStatus> = new Set(["succeeded", "failed", "cancelled"]);
const POLL_INTERVAL_MS = 1500;
const MAX_POLLS = 120;
const MAX_CONSECUTIVE_QUERY_FAILURES = 3;

interface PendingTask {
  task_id: string;
  failures: number;
}

function isTerminal(status: TaskStatus) {
  return TERMINAL_STATUSES.has(status);
}

export function createTaskSettler(onTick: () => void | Promise<void>) {
  let disposed = false;
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const waiters = new Set<() => void>();

  function delay(ms: number) {
    return new Promise<void>((resolve) => {
      const timer = setTimeout(() => {
        timers.delete(timer);
        waiters.delete(resolve);
        resolve();
      }, ms);
      timers.add(timer);
      waiters.add(resolve);
    });
  }

  async function settle(taskRefs: TaskRef[]): Promise<TaskResponse[]> {
    const pending: PendingTask[] = taskRefs
      .filter((task) => task?.task_id)
      .map((task) => ({ task_id: task.task_id, failures: 0 }));
    const finalStates = new Map<string, TaskResponse>();
    if (disposed || pending.length === 0) return [];

    for (let attempt = 0; attempt < MAX_POLLS; attempt += 1) {
      if (disposed) return [...finalStates.values()];
      const fetched = await Promise.all(pending.map(async (item) => {
        try {
          return { item, state: await apiClient.getTask(item.task_id) };
        } catch {
          return { item, state: null };
        }
      }));
      if (disposed) return [...finalStates.values()];

      for (const { state } of fetched) {
        if (state) finalStates.set(state.task_id, state);
      }

      try {
        await onTick();
      } catch {
        // Refresh is best-effort while settling; transient reload failures must
        // not surface as action failures or abort the poll.
      }
      if (disposed) return [...finalStates.values()];

      const nextPending: PendingTask[] = [];
      for (const { item, state } of fetched) {
        if (state) {
          if (!isTerminal(state.status)) nextPending.push({ task_id: item.task_id, failures: 0 });
        } else if (item.failures < MAX_CONSECUTIVE_QUERY_FAILURES) {
          nextPending.push({ task_id: item.task_id, failures: item.failures + 1 });
        }
      }
      if (nextPending.length === 0) return [...finalStates.values()];
      pending.length = 0;
      pending.push(...nextPending);

      await delay(POLL_INTERVAL_MS);
      if (disposed) return [...finalStates.values()];
    }
    return [...finalStates.values()];
  }

  function dispose() {
    disposed = true;
    for (const timer of timers) clearTimeout(timer);
    timers.clear();
    for (const resolve of waiters) resolve();
    waiters.clear();
  }

  return { dispose, settle };
}
