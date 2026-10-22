import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type { TaskResponse } from "@/services/api";

import { errorMessage } from "./settings-error";

const POLL_INTERVAL_MS = 1500;
const ACTIVE_TASK_STATUSES = ["queued", "running", "waiting"];

/**
 * Vector index rebuild status for the runtime section: loads the latest task,
 * polls while it is active, and submits an explicit confirmed rebuild.
 */
export function useVectorRebuild() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<TaskResponse | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(true);

  const showErrorToast = useCallback((error: unknown, fallback: string) => {
    toast.error(t("common.error"), {
      description: errorMessage(error, fallback),
      duration: 5000,
    });
  }, [t]);

  const schedulePoll = useCallback((next: TaskResponse | null) => {
    clearTimeout(timer.current);
    if (!next || !ACTIVE_TASK_STATUSES.includes(next.status)) {
      return;
    }
    timer.current = setTimeout(async () => {
      try {
        const response = await apiClient.listTasks({
          page: 1,
          pageSize: 1,
          view: "processing",
          kind: "vector_rebuild",
          status: null,
          stage: null,
          waitingReason: null,
          dependencyKey: null,
        });
        if (!mounted.current) {
          return;
        }
        const latest = response.items[0] ?? null;
        setStatus(latest);
        schedulePoll(latest);
      } catch (error) {
        showErrorToast(error, t("settings.runtime.vectorRebuildStatusFailed"));
      }
    }, POLL_INTERVAL_MS);
  }, [showErrorToast, t]);

  const load = useCallback(async () => {
    try {
      const response = await apiClient.listTasks({
        page: 1,
        pageSize: 1,
        view: "processing",
        kind: "vector_rebuild",
        status: null,
        stage: null,
        waitingReason: null,
        dependencyKey: null,
      });
      if (!mounted.current) {
        return;
      }
      const latest = response.items[0] ?? null;
      setStatus(latest);
      schedulePoll(latest);
    } catch (error) {
      showErrorToast(error, t("settings.runtime.vectorRebuildStatusFailed"));
    }
  }, [schedulePoll, showErrorToast, t]);

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
      const task = await apiClient.submitVectorIndexRebuild();
      const current = await apiClient.getTask(task.task_id);
      if (!mounted.current) {
        return;
      }
      setStatus(current);
      toast.info(t("settings.runtime.vectorRebuildStarted"), { duration: 2500 });
      schedulePoll(current);
    } catch (error) {
      showErrorToast(error, t("settings.runtime.vectorRebuildFailed"));
    }
  }, [schedulePoll, showErrorToast, t]);

  return {
    confirmOpen,
    setConfirmOpen,
    start,
    status,
    running: status ? ACTIVE_TASK_STATUSES.includes(status.status) : false,
  };
}
