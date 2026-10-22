import { useCallback, useEffect, useRef, useState } from "react";

import type { TaskResponse } from "@/services/api";

import type { QueueScope } from "./queue-updates";
import { createPatchBuffer } from "./queue-updates";
import { useTaskStream } from "./task-stream";

/** Poll interval used only after the SSE transport fails. */
export const LIVE_FALLBACK_INTERVAL_MS = 20_000;
/** One trailing authoritative reload per structural burst. */
export const STRUCTURAL_DEBOUNCE_MS = 1_000;

export type PatchOutcome = "patched" | "stale";

export interface UseQueueLiveOptions {
  /** Whether live updates apply at all; only the processing view is live. */
  enabled: boolean;
  scope: QueueScope;
  /** Patch known rows in the cached window; `stale` asks for a reload. */
  applyPatches: (updates: ReadonlyMap<string, TaskResponse>) => PatchOutcome;
  /** Authoritative reload of the current window (single-flighted upstream). */
  reload: () => void;
  /** Whether a list request is already in flight. */
  isFetching: () => boolean;
  /**
   * Monotonic counter of authoritative list landings for the current window.
   * A frame buffered before the latest landing describes an older server state.
   */
  getAuthoritativeLanding: () => number;
}

export interface QueueLiveState {
  streaming: boolean;
  connected: boolean;
  fallback: boolean;
}

/**
 * Live coordination for the queue: the watch-all snapshot/delta stream, the
 * bounded 100ms patch window, the 1s structural reload debounce, the 20s polling
 * fallback after transport failure, and the hidden-tab pause.
 *
 * A snapshot never replaces the visible window: it patches only rows already in
 * scope, so a completed/trash/filtered/page-2 window and its totals stay
 * authoritative. Frames that change membership or order queue one trailing GET.
 */
export function useQueueLive({
  enabled,
  scope,
  applyPatches,
  reload,
  isFetching,
  getAuthoritativeLanding,
}: UseQueueLiveOptions): QueueLiveState {
  const [state, setState] = useState<QueueLiveState>({
    streaming: false,
    connected: false,
    fallback: false,
  });

  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const applyRef = useRef(applyPatches);
  applyRef.current = applyPatches;
  const reloadRef = useRef(reload);
  reloadRef.current = reload;
  const fetchingRef = useRef(isFetching);
  fetchingRef.current = isFetching;
  const landingRef = useRef(getAuthoritativeLanding);
  landingRef.current = getAuthoritativeLanding;

  const activeRef = useRef(false);
  const syncedRef = useRef(false);
  const bufferRef = useRef(createPatchBuffer());
  // Landing generation captured when each buffered frame was received.
  const frameLandingRef = useRef(new Map<string, number>());
  const patchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const structuralTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fallbackTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const disposedRef = useRef(false);

  const clearPatchTimer = useCallback(() => {
    if (patchTimerRef.current !== null) {
      clearTimeout(patchTimerRef.current);
      patchTimerRef.current = null;
    }
  }, []);

  const clearStructuralTimer = useCallback(() => {
    if (structuralTimerRef.current !== null) {
      clearTimeout(structuralTimerRef.current);
      structuralTimerRef.current = null;
    }
  }, []);

  /** Drop buffered frames and any queued structural reload. */
  const invalidatePending = useCallback(() => {
    bufferRef.current.invalidate();
    frameLandingRef.current = new Map();
    clearPatchTimer();
    clearStructuralTimer();
  }, [clearPatchTimer, clearStructuralTimer]);

  const flushPatches = useCallback(() => {
    patchTimerRef.current = null;
    const updates = bufferRef.current.flush();
    const receivedAt = frameLandingRef.current;
    frameLandingRef.current = new Map();
    if (updates.size === 0) {
      return;
    }
    // A frame that was received before an authoritative read landed describes a
    // state that read has already superseded. Dropping it is not a reason to
    // reload: the cache already holds the newer result, and the structural
    // debounce and polling fallback stay untouched.
    const landing = landingRef.current();
    const fresh = new Map<string, TaskResponse>();
    for (const [taskId, update] of updates) {
      if (receivedAt.get(taskId) === landing) {
        fresh.set(taskId, update);
      }
    }
    if (fresh.size === 0) {
      return;
    }
    if (applyRef.current(fresh) === "stale") {
      invalidatePending();
      scheduleStructuralReloadRef.current();
    }
  }, [invalidatePending]);

  const scheduleStructuralReload = useCallback(() => {
    bufferRef.current.invalidate();
    if (structuralTimerRef.current !== null) {
      return;
    }
    structuralTimerRef.current = setTimeout(() => {
      structuralTimerRef.current = null;
      if (disposedRef.current) {
        return;
      }
      reloadRef.current();
    }, STRUCTURAL_DEBOUNCE_MS);
  }, []);
  const scheduleStructuralReloadRef = useRef(scheduleStructuralReload);
  scheduleStructuralReloadRef.current = scheduleStructuralReload;

  const stopFallback = useCallback(() => {
    if (fallbackTimerRef.current !== null) {
      clearInterval(fallbackTimerRef.current);
      fallbackTimerRef.current = null;
    }
  }, []);

  const startFallback = useCallback(() => {
    stopFallback();
    fallbackTimerRef.current = setInterval(() => {
      if (disposedRef.current) {
        return;
      }
      // A hidden tab costs zero automatic traffic, and an in-flight list request
      // is already fresh: the next tick retries instead of overlapping.
      if (typeof document !== "undefined" && document.visibilityState !== "visible") {
        return;
      }
      if (fetchingRef.current()) {
        return;
      }
      reloadRef.current();
    }, LIVE_FALLBACK_INTERVAL_MS);
  }, [stopFallback]);

  // A row that is not in the window, or that would change membership, queues the
  // single trailing reload. Progress-only frames take the bounded patch path.
  const receiveUpdate = useCallback((task: TaskResponse) => {
    // Deltas that arrive before this connection's snapshot describe a state the
    // client has not synchronised with yet; the snapshot supersedes them.
    if (!syncedRef.current) {
      return;
    }
    if (fetchingRef.current()) {
      invalidatePending();
      reloadRef.current();
      return;
    }
    if (structuralTimerRef.current !== null) {
      return;
    }
    if (bufferRef.current.enqueue(task) === "full") {
      invalidatePending();
      scheduleStructuralReload();
      return;
    }
    frameLandingRef.current.set(task.task_id, landingRef.current());
    if (patchTimerRef.current === null) {
      patchTimerRef.current = setTimeout(() => {
        patchTimerRef.current = null;
        flushPatches();
      }, 100);
    }
  }, [flushPatches, invalidatePending, scheduleStructuralReload]);

  const stream = useTaskStream({
    onSnapshot: (tasks) => {
      syncedRef.current = true;
      stopFallback();
      invalidatePending();
      setState((current) => ({ ...current, connected: true, fallback: false }));
      // Patch only rows the current window already shows; never adopt the
      // processing-first snapshot as the list.
      const known = new Map<string, TaskResponse>();
      for (const task of tasks) {
        known.set(task.task_id, task);
      }
      applyRef.current(known);
      reloadRef.current();
    },
    onUpdate: receiveUpdate,
    onDone: () => {
      invalidatePending();
      setState((current) => ({ ...current, streaming: false, connected: false }));
    },
    onErrorFrame: () => {
      invalidatePending();
      setState((current) => ({ ...current, connected: false }));
    },
    onTransportError: () => {
      invalidatePending();
      setState((current) => ({ ...current, streaming: false, connected: false, fallback: true }));
      if (activeRef.current) {
        startFallback();
      }
      if (syncedRef.current) {
        reloadRef.current();
      }
    },
  });

  const connect = useCallback(() => {
    syncedRef.current = false;
    stream.connect();
    setState((current) => ({ ...current, streaming: true }));
  }, [stream]);

  const disconnect = useCallback(() => {
    stream.disconnect();
    setState({ streaming: false, connected: false, fallback: false });
  }, [stream]);

  // Start/stop follows the active view (only processing is live).
  useEffect(() => {
    if (!enabled) {
      activeRef.current = false;
      syncedRef.current = false;
      stopFallback();
      invalidatePending();
      disconnect();
      return;
    }
    activeRef.current = true;
    invalidatePending();
    if (typeof document !== "undefined" && document.visibilityState === "hidden") {
      return;
    }
    connect();
  }, [connect, disconnect, enabled, invalidatePending, stopFallback]);

  // Hidden tab: close the stream and cancel pending work (zero traffic); visible
  // again: reconnect for a fresh snapshot instead of replaying stale deltas.
  useEffect(() => {
    const onVisibilityChange = () => {
      if (!activeRef.current || disposedRef.current) {
        return;
      }
      invalidatePending();
      if (document.visibilityState === "hidden") {
        // A hidden tab keeps no stream, no poll timer and no pending reload.
        stopFallback();
        disconnect();
      } else {
        connect();
      }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [connect, disconnect, invalidatePending, stopFallback]);

  // Scope changes fence every pending frame: they were resolved against the old
  // window and must never patch the new one.
  useEffect(() => {
    invalidatePending();
  }, [invalidatePending, scope]);

  useEffect(() => () => {
    disposedRef.current = true;
    activeRef.current = false;
    stopFallback();
    invalidatePending();
    stream.disconnect();
  }, [invalidatePending, stopFallback, stream]);

  return state;
}
