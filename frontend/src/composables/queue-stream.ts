import type { Ref } from "vue";

import type { TaskResponse } from "../services/api";
import { createQueueUpdateRegistry, queueUpdateRequiresReload, type QueueUpdateQuery } from "./queue-update-registry";
import { useTaskStream } from "./use-task-stream";

// Live updates (issue 405 Task E3): a watch-all task SSE stream drives
// refreshes instead of the fixed 20s poll. The 20s poll is retained as the
// automatic fallback when the stream errors, is unavailable, or the client
// cannot use cookie-based SSE (e.g. PAT clients): zero behavior loss.
// Issue 730 keeps SSE primary: the fallback starts only after transport/error
// failure, stops after a successful snapshot, and never runs concurrently
// with an in-flight list request.
const LIVE_FALLBACK_INTERVAL_MS = 20_000;

// Structural changes cannot safely patch server-selected membership or order.
// Collapse a burst into one trailing authoritative load.
const STRUCTURAL_DEBOUNCE_MS = 1_000;

export interface UseQueueStreamOptions {
  // The queue's page items ref; coalesced updates replace it immutably.
  items: Ref<TaskResponse[]>;
  // The queue's load function; every snapshot and structural resync calls it.
  // `load` single-flights concurrent triggers, so these calls coalesce rather
  // than overlap.
  load: () => void | Promise<void>;
  // The queue's loading ref; the fallback interval skips a tick while a list
  // request is in flight so fallback polling never overlaps it.
  isLoading?: Ref<boolean>;
  // Current server query, used to distinguish local progress from structural changes.
  query: () => QueueUpdateQuery;
}

/** Live-update coordination for the processing queue: the watch-all SSE
 * deltas, the 20s polling fallback, the 1s structural debounce, and the
 * hidden-tab pause/resume. No queue state is owned here — the caller passes
 * its own `items` ref and `load` function, and the SSE transport stays in
 * `useTaskStream`. The task-event SSE framing and both stream consumers
 * (this watch-all coordinator and the watch-ids task settler) are preserved.
 */
export function useQueueStream({ items, load, isLoading, query }: UseQueueStreamOptions) {
  let liveActive = false;
  let liveSynced = false;
  let fallbackTimer: ReturnType<typeof setInterval> | null = null;
  let structuralTimer: ReturnType<typeof setTimeout> | null = null;

  function cancelStructuralSync() {
    if (structuralTimer !== null) {
      clearTimeout(structuralTimer);
      structuralTimer = null;
    }
  }

  function scheduleStructuralSync() {
    if (structuralTimer !== null) return;
    structuralTimer = setTimeout(() => {
      structuralTimer = null;
      void load();
    }, STRUCTURAL_DEBOUNCE_MS);
  }

  function invalidatePendingUpdates() {
    updateRegistry.invalidate();
    cancelStructuralSync();
  }

  function scheduleStructuralReload() {
    updateRegistry.invalidate();
    scheduleStructuralSync();
  }

  function updateReceivedWhileLoading() {
    invalidatePendingUpdates();
    void load();
  }

  const updateRegistry = createQueueUpdateRegistry({
    items,
    isLoading: () => isLoading?.value ?? false,
    onLoadingUpdate: updateReceivedWhileLoading,
    onStalePage: scheduleStructuralReload,
  });

  function stopFallbackPolling() {
    if (fallbackTimer) {
      clearInterval(fallbackTimer);
      fallbackTimer = null;
    }
  }

  function startFallbackPolling() {
    stopFallbackPolling();
    fallbackTimer = setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
      // Never overlap an in-flight list request: the in-flight snapshot is
      // already fresh, and the next interval tick retries. Immediate resync
      // loads (snapshot/error paths) still go through `load`, which coalesces.
      if (isLoading?.value) return;
      void load();
    }, LIVE_FALLBACK_INTERVAL_MS);
  }

  const taskStream = useTaskStream({
    // Every (re)connect delivers a snapshot first: that snapshot-triggered
    // load() is the one full sync before deltas.
    onSnapshot: () => {
      liveSynced = true;
      stopFallbackPolling();
      invalidatePendingUpdates();
      void load();
    },
    onUpdate: (task) => {
      if (isLoading?.value) {
        updateReceivedWhileLoading();
        return;
      }
      if (structuralTimer !== null) return;
      const current = items.value.find((entry) => entry.task_id === task.task_id);
      if (!current || queueUpdateRequiresReload(current, task, query())) {
        scheduleStructuralReload();
        return;
      }
      updateRegistry.enqueue(task);
    },
    onDone: () => invalidatePendingUpdates(),
    onErrorFrame: () => invalidatePendingUpdates(),
    onTransportError: () => {
      invalidatePendingUpdates();
      if (liveActive) startFallbackPolling();
      if (liveSynced) void load();
    },
  });

  // Open the watch-all stream. Safe to call when SSE is unavailable: the
  // stream reports through the fallback path and the 20s poll takes over.
  // Issue 413 Phase 3: a hidden tab pauses SSE (zero background traffic);
  // the `visibilitychange` resume reconnects for a fresh snapshot resync.
  function startLiveUpdates() {
    if (liveActive) return;
    liveActive = true;
    invalidatePendingUpdates();
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    taskStream.connect();
  }

  function stopLiveUpdates() {
    liveActive = false;
    liveSynced = false;
    stopFallbackPolling();
    invalidatePendingUpdates();
    taskStream.disconnect();
  }

  // Issue 413 Phase 3: pause the SSE stream while the tab is hidden so a
  // background tab costs zero event traffic. The 20s fallback poll already
  // skips hidden tabs; cancel any pending structural sync so no load fires
  // while hidden. On visible, reconnect: the fresh snapshot resyncs updates
  // missed while paused (the server keeps no replay) and its
  // snapshot-triggered load() is the catch-up.
  function handleVisibilityChange() {
    if (!liveActive || typeof document === "undefined") return;
    invalidatePendingUpdates();
    if (document.visibilityState === "hidden") {
      taskStream.disconnect();
    } else {
      taskStream.reconnect();
    }
  }

  // Unmount teardown for the live machinery: same order the queue's
  // onBeforeUnmount used, and it keeps `liveSynced` untouched (the caller
  // aborts in-flight requests right after).
  function disposeLiveUpdates() {
    liveActive = false;
    stopFallbackPolling();
    invalidatePendingUpdates();
    taskStream.disconnect();
  }

  return {
    taskStream,
    invalidatePendingUpdates,
    startLiveUpdates,
    stopLiveUpdates,
    handleVisibilityChange,
    disposeLiveUpdates,
  };
}
