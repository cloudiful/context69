import type { Ref } from "vue";

import type { TaskResponse } from "../services/api";
import { useTaskStream } from "./use-task-stream";

// Live updates (issue 405 Task E3): a watch-all task SSE stream drives
// refreshes instead of the fixed 20s poll. The 20s poll is retained as the
// automatic fallback when the stream errors, is unavailable, or the client
// cannot use cookie-based SSE (e.g. PAT clients): zero behavior loss.
// Issue 730 keeps SSE primary: the fallback starts only after transport/error
// failure, stops after a successful snapshot, and never runs concurrently
// with an in-flight list request.
const LIVE_FALLBACK_INTERVAL_MS = 20_000;

// Issue 408 Task F1: off-page updates (task_id not in the current page) imply a
// structural change (new/vanished task, shifted totals) that only a full load
// can reflect. Collapse a burst of such frames into one trailing load so a
// 2781-item storm costs one request, not one per frame.
const STRUCTURAL_DEBOUNCE_MS = 1_000;

export interface UseQueueStreamOptions {
  // The queue's page items ref; on-page deltas merge in place.
  items: Ref<TaskResponse[]>;
  // The queue's load function; every snapshot and structural resync calls it.
  // `load` single-flights concurrent triggers, so these calls coalesce rather
  // than overlap.
  load: () => void | Promise<void>;
  // The queue's loading ref; the fallback interval skips a tick while a list
  // request is in flight so fallback polling never overlaps it.
  isLoading?: Ref<boolean>;
}

/** Live-update coordination for the processing queue: the watch-all SSE
 * deltas, the 20s polling fallback, the 1s structural debounce, and the
 * hidden-tab pause/resume. No queue state is owned here — the caller passes
 * its own `items` ref and `load` function, and the SSE transport stays in
 * `useTaskStream`. The task-event SSE framing and both stream consumers
 * (this watch-all coordinator and the watch-ids task settler) are preserved.
 */
export function useQueueStream({ items, load, isLoading }: UseQueueStreamOptions) {
  let liveActive = false;
  let liveSynced = false;
  let fallbackTimer: ReturnType<typeof setInterval> | null = null;
  let structuralTimer: ReturnType<typeof setTimeout> | null = null;

  function cancelStructuralSync() {
    if (structuralTimer) {
      clearTimeout(structuralTimer);
      structuralTimer = null;
    }
  }

  function scheduleStructuralSync() {
    if (structuralTimer) return;
    structuralTimer = setTimeout(() => {
      structuralTimer = null;
      void load();
    }, STRUCTURAL_DEBOUNCE_MS);
  }

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
      cancelStructuralSync();
      void load();
    },
    // Issue 408 Task F1: an update frame carries the task's current full
    // state. When its task_id is already on the current page, merge it in
    // place (status/stage/progress/counts/error stay realtime) with zero
    // requests. Otherwise it signals a structural change: collapse the burst
    // into one trailing debounced load (~1s).
    onUpdate: (task) => {
      const index = items.value.findIndex((entry) => entry.task_id === task.task_id);
      if (index >= 0) {
        items.value[index] = { ...items.value[index], ...task };
        return;
      }
      scheduleStructuralSync();
    },
    onDone: () => {
      // Watch-all subscriptions never emit `done`; ignore defensively.
    },
    onErrorFrame: () => {
      if (liveActive) startFallbackPolling();
      if (liveSynced) {
        cancelStructuralSync();
        void load();
      }
    },
    onTransportError: () => {
      if (liveActive) startFallbackPolling();
      // Resync only when the broken stream had delivered its snapshot: before
      // the first snapshot no delta could have been missed, and the mount
      // load plus the fallback poll already cover freshness.
      if (liveSynced) {
        cancelStructuralSync();
        void load();
      }
    },
  });

  // Open the watch-all stream. Safe to call when SSE is unavailable: the
  // stream reports through the fallback path and the 20s poll takes over.
  // Issue 413 Phase 3: a hidden tab pauses SSE (zero background traffic);
  // the `visibilitychange` resume reconnects for a fresh snapshot resync.
  function startLiveUpdates() {
    if (liveActive) return;
    liveActive = true;
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    taskStream.connect();
  }

  function stopLiveUpdates() {
    liveActive = false;
    liveSynced = false;
    stopFallbackPolling();
    cancelStructuralSync();
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
    if (document.visibilityState === "hidden") {
      cancelStructuralSync();
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
    cancelStructuralSync();
    taskStream.disconnect();
  }

  return {
    taskStream,
    startLiveUpdates,
    stopLiveUpdates,
    handleVisibilityChange,
    disposeLiveUpdates,
  };
}
