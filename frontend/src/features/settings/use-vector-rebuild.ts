import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { useAuthSession } from "@/hooks/use-auth";
import { apiClient } from "@/services/api";
import type { VectorIndexRebuildState, VectorIndexRebuildStatus } from "@/services/api";

import { errorMessage } from "./settings-error";

const POLL_INTERVAL_MS = 1500;
const SLOW_POLL_INTERVAL_MS = 5000;
/** Bounded retries after a transient status read failure (five attempts). */
const MAX_POLL_RETRIES = 5;

/**
 * Whether a failed status read may be retried after `attempt` failures.
 *
 * The retry is bounded so a permanently unavailable endpoint does not poll
 * forever; the last observed status stays on screen and the manual controls
 * reread on demand.
 */
export function shouldRetryStatusPoll(attempt: number): boolean {
  return attempt + 1 < MAX_POLL_RETRIES;
}

/** The states in which the rebuild still has work to report. */
const POLLED_STATES: VectorIndexRebuildState[] = ["queued", "running", "pausing", "paused"];

/** The states that still keep the operator waiting for the rebuild. */
const RUNNING_STATES: VectorIndexRebuildState[] = ["queued", "running", "pausing", "paused"];

/** The states in which the rebuild owns the durable rebuild barrier. */
const BARRIER_STATES: VectorIndexRebuildState[] = ["running", "pausing", "paused"];

let lastObservedStatus: VectorIndexRebuildStatus | null = null;

/**
 * The last rebuild status this page observed.
 *
 * The settings save reads it to refuse an embedding identity change while the
 * rebuild owns the barrier: the backend refuses that save anyway, and the
 * operator is told to cancel the rebuild instead of meeting a bare conflict.
 */
export function lastObservedRebuildStatus(): VectorIndexRebuildStatus | null {
  return lastObservedStatus;
}

/** Whether a rebuild currently owns the durable rebuild barrier. */
export function rebuildHoldsBarrier(status: VectorIndexRebuildStatus | null): boolean {
  return status !== null && BARRIER_STATES.includes(status.state);
}

function isPolling(status: VectorIndexRebuildStatus | null): boolean {
  return status !== null && POLLED_STATES.includes(status.state);
}

function pollDelay(status: VectorIndexRebuildStatus | null): number {
  // A paused rebuild waits for an operator, so it is polled slowly instead of
  // not at all: another operator may resume or cancel it.
  return status?.state === "paused" ? SLOW_POLL_INTERVAL_MS : POLL_INTERVAL_MS;
}

/**
 * Durable vector index rebuild status for the runtime section.
 *
 * The status is the typed projection of the backend's durable rows, so it
 * survives a restart and never describes an in-process progress guess. Every
 * control carries the observed task id, so it can only act on the rebuild the
 * operator saw. The rebuild surface is admin-only, so a non-admin viewer never
 * issues the admin read.
 */
export function useVectorRebuild() {
  const { t } = useTranslation();
  const { user } = useAuthSession();
  const isAdmin = user?.is_admin === true;
  const [status, setStatusState] = useState<VectorIndexRebuildStatus | null>(null);
  const setStatus = useCallback((next: VectorIndexRebuildStatus | null) => {
    lastObservedStatus = next;
    setStatusState(next);
  }, []);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(true);

  const showErrorToast = useCallback((error: unknown, fallback: string) => {
    toast.error(t("common.error"), {
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  }, [t]);

  /**
   * Read the durable status once, then schedule the next read while the
   * rebuild still has work. A transient read failure keeps the last observed
   * status, shows one actionable error and retries a bounded number of times;
   * a recovered read after an external resume or cancel replaces the stale
   * status and stops polling when the rebuild is terminal.
   */
  const fetchStatus = useCallback(async (attempt: number, delayStatus: VectorIndexRebuildStatus | null) => {
    try {
      const response = await apiClient.getVectorIndexRebuildStatus();
      if (!mounted.current) {
        return;
      }
      setStatus(response);
      if (isPolling(response)) {
        timer.current = setTimeout(() => {
          void fetchStatus(0, response);
        }, pollDelay(response));
      }
    } catch (error) {
      if (!mounted.current) {
        return;
      }
      if (attempt === 0) {
        showErrorToast(error, t("settings.runtime.vectorRebuildStatusFailed"));
      }
      if (shouldRetryStatusPoll(attempt)) {
        timer.current = setTimeout(() => {
          void fetchStatus(attempt + 1, delayStatus);
        }, pollDelay(delayStatus));
      }
    }
  }, [setStatus, showErrorToast, t]);

  const schedulePoll = useCallback((next: VectorIndexRebuildStatus | null) => {
    clearTimeout(timer.current);
    if (!isPolling(next)) {
      return;
    }
    timer.current = setTimeout(() => {
      void fetchStatus(0, next);
    }, pollDelay(next));
  }, [fetchStatus]);

  const load = useCallback(async () => {
    clearTimeout(timer.current);
    if (!isAdmin) {
      lastObservedStatus = null;
      setStatusState(null);
      return;
    }
    await fetchStatus(0, null);
  }, [fetchStatus, isAdmin]);

  useEffect(() => {
    mounted.current = true;
    void load();
    return () => {
      mounted.current = false;
      clearTimeout(timer.current);
    };
  }, [load]);

  const start = useCallback(async () => {
    try {
      await apiClient.submitVectorIndexRebuild();
      if (!mounted.current) {
        return;
      }
      toast.info(t("settings.runtime.vectorRebuildStarted"), { duration: 2500 });
      await load();
    } catch (error) {
      showErrorToast(error, t("settings.runtime.vectorRebuildFailed"));
    }
  }, [load, showErrorToast, t]);

  const control = useCallback(async (
    action: "pause" | "resume" | "cancel",
    taskId: string,
  ) => {
    setPending(action);
    try {
      const response = action === "pause"
        ? await apiClient.pauseVectorIndexRebuild(taskId)
        : action === "resume"
          ? await apiClient.resumeVectorIndexRebuild(taskId)
          : await apiClient.cancelVectorIndexRebuild(taskId);
      if (!mounted.current) {
        return;
      }
      setStatus(response);
      schedulePoll(response);
      const message = action === "pause"
        ? t("settings.runtime.vectorRebuildPauseRequested")
        : action === "resume"
          ? t("settings.runtime.vectorRebuildResumed")
          : t("settings.runtime.vectorRebuildCancelled");
      toast.info(message, { duration: 3000 });
    } catch (error) {
      showErrorToast(error, t("settings.runtime.vectorRebuildControlFailed"));
      // The refusal is the backend's authority; rereading the status shows the
      // state the control actually met.
      await load();
    } finally {
      if (mounted.current) {
        setPending(null);
      }
    }
  }, [load, schedulePoll, setStatus, showErrorToast, t]);

  const pause = useCallback(async () => {
    if (status?.task_id) {
      await control("pause", status.task_id);
    }
  }, [control, status?.task_id]);

  const resume = useCallback(async () => {
    if (status?.task_id) {
      await control("resume", status.task_id);
    }
  }, [control, status?.task_id]);

  const cancel = useCallback(async () => {
    if (status?.task_id) {
      await control("cancel", status.task_id);
    }
  }, [control, status?.task_id]);

  return {
    confirmOpen,
    setConfirmOpen,
    start,
    status,
    isAdmin,
    pending,
    pause,
    resume,
    cancel,
    /** Whether the rebuild still has work the operator waits for. */
    running: status !== null && RUNNING_STATES.includes(status.state),
    /** Whether a rebuild owns the durable rebuild barrier right now. */
    busy: rebuildHoldsBarrier(status),
  };
}
