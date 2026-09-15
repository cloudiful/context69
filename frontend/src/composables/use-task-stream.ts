import { onBeforeUnmount, ref } from "vue";

import type { TaskResponse } from "../services/api";
import { resolveApiUrl } from "../services/api/api-core";

/** Build the GET /v1/tasks/stream URL. Empty/absent ids means watch-all
 * (the snapshot covers the `processing` view, first 100; deltas cover every
 * own task). Explicit ids watch only those tasks (own-tasks-only filter).
 */
export function buildTaskStreamUrl(taskIds?: string[]): string {
  const base = resolveApiUrl("/v1/tasks/stream");
  const ids = (taskIds ?? []).map((id) => id.trim()).filter((id) => id.length > 0);
  if (ids.length === 0) return base;
  const params = new URLSearchParams();
  params.set("task_ids", ids.join(","));
  return `${base}?${params.toString()}`;
}

export interface TaskStreamHooks {
  /** First frame on every connection: full states at subscribe time.
   * Callers render this snapshot, then apply `update` deltas. Reconnects get
   * a fresh snapshot; there is no server replay via `Last-Event-ID`.
   */
  onSnapshot: (tasks: TaskResponse[]) => void;
  /** Incremental frame: one watched task's current full state. */
  onUpdate: (task: TaskResponse) => void;
  /** Terminal frame (explicit `task_ids` watch only): every watched task is
   * terminal. Watch-all streams never emit `done`.
   */
  onDone?: (tasks: TaskResponse[]) => void;
  /** In-band `error` frame: the server asks the client to resync via GET and
   * reconnect (e.g. lagged broadcast). The stream is already closed; the
   * caller must fall back to polling.
   */
  onErrorFrame?: (message: string) => void;
  /** Transport failure: EventSource unavailable, constructor threw (PAT
   * clients cannot send an Authorization header over EventSource), or a
   * network-level `onerror`. The caller must fall back to polling with zero
   * behavior loss.
   */
  onTransportError?: () => void;
}

export interface UseTaskStream {
  /** True while an EventSource instance is open. */
  streaming: ReturnType<typeof ref<boolean>>;
  /** True once the current connection delivered its snapshot (the one full
   * sync before deltas). False before the snapshot or after close/error.
   */
  connected: ReturnType<typeof ref<boolean>>;
  /** True after the stream failed and the caller should poll. Reset on the
   * next successful `connect`.
   */
  usingFallback: ReturnType<typeof ref<boolean>>;
  /** Open a subscription. Reconnects get a fresh snapshot first; deltas only
   * apply after that snapshot. A failed open reports through
   * `onTransportError` for polling fallback instead of throwing.
   */
  connect: (taskIds?: string[]) => void;
  /** Close the active subscription (new subscription or unmount). */
  disconnect: () => void;
  /** Reopen the last subscription (same task ids). Used for hidden-tab
   * pause/resume so a background tab costs zero SSE traffic and resyncs via
   * a fresh snapshot on return.
   */
  reconnect: () => void;
}

/** Long-lived SSE subscription for GET /v1/tasks/stream.
 *
 * Mirrors `use-search-stream.ts`: `EventSource` with `withCredentials: true`,
 * named-frame listeners (`snapshot`/`update`/`done`/`error`), transport
 * `onerror` fallback, detached handlers before `close()`, and FakeEventSource
 * test doubles. Unlike the search one-shot, this stays open: watch-all
 * subscriptions never emit `done` and close only on client disconnect.
 *
 * Issue 413 Phase 2: the server coalesces `update` frames per task every 3s
 * (latest-wins, snapshot stays immediate). The client therefore merges each
 * `update` in place with no extra throttle or debounce: at most one frame per
 * task per 3s window keeps traffic an order of magnitude below the
 * unthrottled storm.
 */
export function useTaskStream(hooks: TaskStreamHooks): UseTaskStream {
  const streaming = ref(false);
  const connected = ref(false);
  const usingFallback = ref(false);

  let stream: EventSource | null = null;
  let lastTaskIds: string[] | undefined;

  function teardownStream() {
    if (stream) {
      // Detach every handler first so close() cannot fire the error fallback.
      stream.onopen = null;
      stream.onerror = null;
      stream.onmessage = null;
      stream.close();
      stream = null;
    }
  }

  function failToPolling() {
    teardownStream();
    streaming.value = false;
    connected.value = false;
    usingFallback.value = true;
    hooks.onTransportError?.();
  }

  function disconnect() {
    teardownStream();
    streaming.value = false;
    connected.value = false;
  }

  function parseTasks(raw: string): TaskResponse[] | null {
    try {
      const payload = JSON.parse(raw) as { tasks?: unknown };
      if (!payload || !Array.isArray(payload.tasks)) return null;
      return payload.tasks as TaskResponse[];
    } catch {
      return null;
    }
  }

  function parseTask(raw: string): TaskResponse | null {
    try {
      const payload = JSON.parse(raw) as { task?: unknown };
      if (!payload || typeof payload.task !== "object" || payload.task === null) return null;
      return payload.task as TaskResponse;
    } catch {
      return null;
    }
  }

  function connect(taskIds?: string[]) {
    disconnect();
    usingFallback.value = false;
    lastTaskIds = taskIds ? [...taskIds] : undefined;

    if (typeof EventSource !== "function") {
      // EventSource unavailable (or PAT-style clients that cannot use
      // cookie-based SSE): fall back to polling with zero behavior loss.
      failToPolling();
      return;
    }

    let es: EventSource;
    try {
      es = new EventSource(buildTaskStreamUrl(taskIds), { withCredentials: true });
    } catch {
      failToPolling();
      return;
    }
    stream = es;
    streaming.value = true;

    es.addEventListener("snapshot", (event) => {
      if (es !== stream) return;
      const tasks = parseTasks((event as MessageEvent).data);
      if (!tasks) return;
      connected.value = true;
      hooks.onSnapshot(tasks);
    });
    es.addEventListener("update", (event) => {
      if (es !== stream) return;
      const task = parseTask((event as MessageEvent).data);
      if (!task) return;
      hooks.onUpdate(task);
    });
    es.addEventListener("done", (event) => {
      if (es !== stream) return;
      const tasks = parseTasks((event as MessageEvent).data) ?? [];
      disconnect();
      hooks.onDone?.(tasks);
    });
    es.addEventListener("error", (event) => {
      // In-band server error frame: resync via GET and stay on polling.
      if (es !== stream) return;
      let message = "task stream error";
      try {
        const payload = JSON.parse((event as MessageEvent).data) as { message?: unknown };
        if (typeof payload.message === "string" && payload.message) message = payload.message;
      } catch {
        // Keep the default message for malformed frames.
      }
      failToPolling();
      hooks.onErrorFrame?.(message);
    });
    es.onerror = () => {
      // Network-level failure (or server close without `done`).
      if (es !== stream) return;
      failToPolling();
    };
  }

  onBeforeUnmount(() => {
    disconnect();
  });

  return {
    streaming,
    connected,
    usingFallback,
    connect,
    disconnect,
    reconnect: () => connect(lastTaskIds),
  };
}
