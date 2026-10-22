import { useCallback, useEffect, useMemo, useRef } from "react";

import { resolveApiUrl } from "@/services/api/api-core";
import type { TaskResponse } from "@/services/api";

/**
 * Builds the watch-all task stream URL. No `task_ids` means the snapshot covers
 * the processing view's first page while deltas cover every own task.
 */
export function buildTaskStreamUrl(taskIds?: string[]): string {
  const base = resolveApiUrl("/v1/tasks/stream");
  const ids = (taskIds ?? []).map((id) => id.trim()).filter(Boolean);
  if (ids.length === 0) {
    return base;
  }
  const params = new URLSearchParams();
  params.set("task_ids", ids.join(","));
  return `${base}?${params.toString()}`;
}

export interface TaskStreamHooks {
  /** First frame on every connection. Reconnects get a fresh one; the server
   * replays nothing, so no Last-Event-ID is assumed. */
  onSnapshot: (tasks: TaskResponse[]) => void;
  onUpdate: (task: TaskResponse) => void;
  onDone?: (tasks: TaskResponse[]) => void;
  onErrorFrame?: (message: string) => void;
  onTransportError?: () => void;
}

export interface TaskStreamController {
  connect: () => void;
  disconnect: () => void;
  reconnect: () => void;
  isOpen: () => boolean;
}

function parseTasks(raw: string): TaskResponse[] | null {
  try {
    const payload = JSON.parse(raw) as { tasks?: unknown };
    return payload && Array.isArray(payload.tasks) ? payload.tasks as TaskResponse[] : null;
  } catch {
    return null;
  }
}

function parseTask(raw: string): TaskResponse | null {
  try {
    const payload = JSON.parse(raw) as { task?: unknown };
    return payload && typeof payload.task === "object" && payload.task !== null
      ? payload.task as TaskResponse
      : null;
  } catch {
    return null;
  }
}

/**
 * Long-lived cookie-authenticated SSE subscription for the queue. Named frames
 * (`snapshot`/`update`/`done`/`error`) drive the caller; transport failures are
 * reported once for polling fallback. Every handler is detached and the source
 * closed together, so no handler leaks across a reconnect or unmount.
 */
export function createTaskStream(hooks: TaskStreamHooks): TaskStreamController {
  let stream: EventSource | null = null;
  let detach: (() => void) | null = null;

  /** Detach named listeners, then detach transport handlers, then close. */
  function teardown() {
    const current = stream;
    const detachCurrent = detach;
    stream = null;
    detach = null;
    if (detachCurrent) {
      detachCurrent();
    }
    if (!current) {
      return;
    }
    current.onopen = null;
    current.onerror = null;
    current.onmessage = null;
    current.close();
  }

  function failToPolling() {
    teardown();
    hooks.onTransportError?.();
  }

  function connect() {
    teardown();
    if (typeof EventSource !== "function") {
      failToPolling();
      return;
    }

    let source: EventSource;
    try {
      source = new EventSource(buildTaskStreamUrl(), { withCredentials: true });
    } catch {
      failToPolling();
      return;
    }
    stream = source;

    const listeners: Array<[string, (event: Event) => void]> = [
      ["snapshot", (event) => {
        if (source !== stream) {
          return;
        }
        const tasks = parseTasks((event as MessageEvent).data);
        if (tasks) {
          hooks.onSnapshot(tasks);
        }
      }],
      ["update", (event) => {
        if (source !== stream) {
          return;
        }
        const task = parseTask((event as MessageEvent).data);
        if (task) {
          hooks.onUpdate(task);
        }
      }],
      ["done", (event) => {
        if (source !== stream) {
          return;
        }
        const tasks = parseTasks((event as MessageEvent).data) ?? [];
        teardown();
        hooks.onDone?.(tasks);
      }],
      ["error", (event) => {
        if (source !== stream) {
          return;
        }
        let message = "task stream error";
        try {
          const payload = JSON.parse((event as MessageEvent).data) as { message?: unknown };
          if (typeof payload.message === "string" && payload.message) {
            message = payload.message;
          }
        } catch {
          // Keep the default message for a malformed frame.
        }
        failToPolling();
        hooks.onErrorFrame?.(message);
      }],
    ];
    for (const [type, listener] of listeners) {
      source.addEventListener(type, listener);
    }
    detach = () => {
      for (const [type, listener] of listeners) {
        source.removeEventListener(type, listener);
      }
    };
    source.onerror = () => {
      if (source !== stream) {
        return;
      }
      failToPolling();
    };
  }

  return {
    connect,
    disconnect: teardown,
    reconnect: connect,
    isOpen: () => stream !== null,
  };
}

/** React binding that owns one controller and disposes it with the component. */
export function useTaskStream(hooks: TaskStreamHooks): TaskStreamController {
  const hooksRef = useRef(hooks);
  hooksRef.current = hooks;
  const controllerRef = useRef<TaskStreamController | null>(null);

  if (!controllerRef.current) {
    controllerRef.current = createTaskStream({
      onSnapshot: (tasks) => hooksRef.current.onSnapshot(tasks),
      onUpdate: (task) => hooksRef.current.onUpdate(task),
      onDone: (tasks) => hooksRef.current.onDone?.(tasks),
      onErrorFrame: (message) => hooksRef.current.onErrorFrame?.(message),
      onTransportError: () => hooksRef.current.onTransportError?.(),
    });
  }

  useEffect(() => () => {
    controllerRef.current?.disconnect();
  }, []);

  const connect = useCallback(() => controllerRef.current?.connect(), []);
  const disconnect = useCallback(() => controllerRef.current?.disconnect(), []);
  const reconnect = useCallback(() => controllerRef.current?.reconnect(), []);
  const isOpen = useCallback(() => controllerRef.current?.isOpen() ?? false, []);

  // One stable controller identity: consumers wire start/stop effects to these
  // callbacks, so a new object per render would tear the stream down repeatedly.
  return useMemo(
    () => ({ connect, disconnect, reconnect, isOpen }),
    [connect, disconnect, reconnect, isOpen],
  );
}
