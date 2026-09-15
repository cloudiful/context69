import { apiClient, type TaskRef, type TaskResponse, type TaskStatus } from "../services/api";

import { buildTaskStreamUrl } from "./use-task-stream";

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

  // Stream-first acceleration (issue 405 Task E3): one shared watch-ids SSE
  // subscription covers the union of pending tasks. The polling loop below is
  // retained unchanged as the fallback: when the stream errors, is
  // unavailable, or the client cannot use cookie-based SSE (e.g. PAT
  // clients), polling continues with zero behavior loss. Stream updates only
  // resolve early; they never change the polling bookkeeping.
  let stream: EventSource | null = null;
  let streamFailed = false;
  let streamWatchKey = "";

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

  async function notifyTick() {
    try {
      await onTick();
    } catch {
      // Refresh is best-effort while settling; transient reload failures must
      // not surface as action failures or abort the settle.
    }
  }

  function teardownStream() {
    if (stream) {
      // Detach every handler first so close() cannot fire the error fallback.
      stream.onopen = null;
      stream.onerror = null;
      stream.onmessage = null;
      stream.close();
      stream = null;
    }
    streamWatchKey = "";
  }

  function failStreamToPolling() {
    streamFailed = true;
    teardownStream();
  }

  function parseSnapshot(raw: string): TaskResponse[] | null {
    try {
      const payload = JSON.parse(raw) as { tasks?: unknown };
      if (!payload || !Array.isArray(payload.tasks)) return null;
      return payload.tasks as TaskResponse[];
    } catch {
      return null;
    }
  }

  function parseUpdate(raw: string): TaskResponse | null {
    try {
      const payload = JSON.parse(raw) as { task?: unknown };
      if (!payload || typeof payload.task !== "object" || payload.task === null) return null;
      return payload.task as TaskResponse;
    } catch {
      return null;
    }
  }

  // Every (re)connect delivers a snapshot first: applying it is the one full
  // sync before deltas. Deltas only resolve tasks already covered by that
  // sync, so a missed broadcast between settle() and stream open cannot skip
  // a terminal state (the snapshot already carried it). State changes apply
  // synchronously so an in-flight polling round cannot overwrite a newer
  // stream state; the refresh tick stays best-effort and never gates the
  // resolve.
  function applySnapshotTasks(tasks: TaskResponse[]) {
    if (disposed) return;
    let touched = false;
    for (const task of tasks) {
      if (!task || typeof task.task_id !== "string") continue;
      if (!pendingTasks.has(task.task_id)) continue;
      finalStates.set(task.task_id, task);
      touched = true;
    }
    if (!touched) return;
    for (const task of tasks) {
      if (!task || typeof task.task_id !== "string") continue;
      if (!pendingTasks.has(task.task_id)) continue;
      if (isTerminal(task.status)) {
        pendingTasks.delete(task.task_id);
      }
    }
    settleIdleCallers();
    if (pendingTasks.size === 0) teardownStream();
    void notifyTick();
  }

  function applyUpdateTask(task: TaskResponse) {
    if (disposed) return;
    if (!task || typeof task.task_id !== "string") return;
    if (!pendingTasks.has(task.task_id)) return;
    finalStates.set(task.task_id, task);
    if (isTerminal(task.status)) {
      pendingTasks.delete(task.task_id);
      settleIdleCallers();
      if (pendingTasks.size === 0) teardownStream();
    }
    void notifyTick();
  }

  function openStreamIfNeeded() {
    if (disposed || pendingTasks.size === 0) {
      teardownStream();
      return;
    }
    if (streamFailed) return;
    if (typeof EventSource !== "function") {
      streamFailed = true;
      return;
    }
    const ids = [...pendingTasks.keys()].sort();
    const key = ids.join(",");
    if (stream && streamWatchKey === key) return;

    teardownStream();
    let es: EventSource;
    try {
      es = new EventSource(buildTaskStreamUrl(ids), { withCredentials: true });
    } catch {
      streamFailed = true;
      return;
    }
    stream = es;
    streamWatchKey = key;

    es.addEventListener("snapshot", (event) => {
      if (es !== stream || disposed) return;
      const tasks = parseSnapshot((event as MessageEvent).data);
      if (!tasks) return;
      applySnapshotTasks(tasks);
    });
    es.addEventListener("update", (event) => {
      if (es !== stream || disposed) return;
      const task = parseUpdate((event as MessageEvent).data);
      if (!task) return;
      applyUpdateTask(task);
    });
    es.addEventListener("done", (event) => {
      if (es !== stream || disposed) return;
      const tasks = parseSnapshot((event as MessageEvent).data) ?? [];
      for (const task of tasks) {
        if (!task || typeof task.task_id !== "string") continue;
        if (!pendingTasks.has(task.task_id)) continue;
        finalStates.set(task.task_id, task);
        pendingTasks.delete(task.task_id);
      }
      settleIdleCallers();
      teardownStream();
    });
    es.addEventListener("error", () => {
      // In-band server error frame: resync via GET and stay on polling.
      if (es !== stream || disposed) return;
      failStreamToPolling();
    });
    es.onerror = () => {
      if (es !== stream || disposed) return;
      failStreamToPolling();
    };
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
          // Skip tasks the stream already settled: a newer stream state must
          // never be overwritten by a stale in-flight poll.
          if (state && pendingTasks.has(task_id)) finalStates.set(task_id, state);
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
    let joinedNewTask = false;
    for (const taskId of ids) {
      if (!pendingTasks.has(taskId)) {
        pendingTasks.set(taskId, { failures: 0, polls: 0 });
        finalStates.delete(taskId);
        joinedNewTask = true;
      }
    }
    ensureLoop();
    // A new task expands the watched union: give the stream another chance
    // with a fresh snapshot even after a past failure.
    if (joinedNewTask && streamFailed) streamFailed = false;
    openStreamIfNeeded();
    return promise;
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    teardownStream();
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
