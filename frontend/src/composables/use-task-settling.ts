import { apiClient, type TaskRef, type TaskResponse, type TaskStatus } from "../services/api";

const TERMINAL_STATUSES: ReadonlySet<TaskStatus> = new Set(["succeeded", "failed", "cancelled"]);
const POLL_INTERVAL_MS = 1500;
const MAX_POLLS = 120;
const MAX_CONSECUTIVE_QUERY_FAILURES = 3;

interface TaskEntry {
  failures: number;
  polls: number;
}

interface Caller {
  ids: string[];
  resolve: (results: TaskResponse[]) => void;
}

function isTerminal(status: TaskStatus) {
  return TERMINAL_STATUSES.has(status);
}

export function createTaskSettler(onTick: () => void | Promise<void>) {
  let disposed = false;
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const waiters = new Set<() => void>();
  const pendingTasks = new Map<string, TaskEntry>();
  const finalStates = new Map<string, TaskResponse>();
  const callers = new Set<Caller>();
  let loopRunning = false;

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

  function snapshotCallerResults(ids: string[]): TaskResponse[] {
    const results: TaskResponse[] = [];
    for (const taskId of ids) {
      const state = finalStates.get(taskId);
      if (state) results.push(state);
    }
    return results;
  }

  function settleIdleCallers() {
    for (const caller of [...callers]) {
      const done = caller.ids.every((taskId) => !pendingTasks.has(taskId));
      if (done) {
        callers.delete(caller);
        caller.resolve(snapshotCallerResults(caller.ids));
      }
    }
  }

  async function runLoop() {
    try {
      while (!disposed && pendingTasks.size > 0) {
        await Promise.resolve();
        if (disposed || pendingTasks.size === 0) break;
        const snapshot = [...pendingTasks.keys()];
        const fetched = await Promise.all(snapshot.map(async (task_id) => {
          try {
            return { task_id, state: await apiClient.getTask(task_id) };
          } catch {
            return { task_id, state: null as TaskResponse | null };
          }
        }));
        if (disposed) break;

        for (const { task_id, state } of fetched) {
          if (state) finalStates.set(task_id, state);
        }

        try {
          await onTick();
        } catch {
          // Refresh is best-effort while settling; transient reload failures must
          // not surface as action failures or abort the poll.
        }
        if (disposed) break;

        for (const { task_id, state } of fetched) {
          const entry = pendingTasks.get(task_id);
          if (!entry) continue;
          entry.polls += 1;
          if (state) {
            if (isTerminal(state.status) || entry.polls >= MAX_POLLS) {
              pendingTasks.delete(task_id);
            } else {
              entry.failures = 0;
            }
          } else {
            entry.failures += 1;
            if (entry.failures > MAX_CONSECUTIVE_QUERY_FAILURES || entry.polls >= MAX_POLLS) {
              pendingTasks.delete(task_id);
            }
          }
        }
        settleIdleCallers();
        if (disposed || pendingTasks.size === 0) break;

        await delay(POLL_INTERVAL_MS);
      }
    } finally {
      loopRunning = false;
    }
  }

  function ensureLoop() {
    if (loopRunning) return;
    loopRunning = true;
    void runLoop();
  }

  function settle(taskRefs: TaskRef[]): Promise<TaskResponse[]> {
    const ids: string[] = [];
    const seen = new Set<string>();
    for (const task of taskRefs) {
      const taskId = task?.task_id;
      if (!taskId || seen.has(taskId)) continue;
      seen.add(taskId);
      ids.push(taskId);
    }
    if (disposed || ids.length === 0) return Promise.resolve([]);

    let resolveFn!: (results: TaskResponse[]) => void;
    const promise = new Promise<TaskResponse[]>((resolve) => {
      resolveFn = resolve;
    });
    const caller: Caller = { ids, resolve: resolveFn };
    callers.add(caller);
    for (const taskId of ids) {
      if (!pendingTasks.has(taskId)) {
        pendingTasks.set(taskId, { failures: 0, polls: 0 });
        finalStates.delete(taskId);
      }
    }
    ensureLoop();
    return promise;
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    for (const timer of timers) clearTimeout(timer);
    timers.clear();
    for (const resolve of waiters) resolve();
    waiters.clear();
    for (const caller of [...callers]) {
      callers.delete(caller);
      caller.resolve(snapshotCallerResults(caller.ids));
    }
    pendingTasks.clear();
  }

  return { dispose, settle };
}
