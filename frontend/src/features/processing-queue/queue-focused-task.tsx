import { X } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import { Button } from "@/components/ui/button";
import type { TaskResponse, TaskStatus } from "@/services/api";

import { QueueExpandedRow } from "./queue-expanded-row";
import { isActiveTask, queueStageLabel, taskSubject } from "./queue-helpers";
import { isValidTaskFocusId } from "./use-queue";

function statusTone(status: TaskStatus): StatusTone {
  switch (status) {
    case "queued":
      return "info";
    case "running":
      return "active";
    case "waiting":
      return "warning";
    case "succeeded":
      return "success";
    case "failed":
      return "failure";
    default:
      return "neutral";
  }
}

/**
 * One focused task reached through `?task=<uuid>`: a bounded card built from
 * the same expanded-row conventions as the table, shown even when the task is
 * on another page or in another view. The detail read itself lives in
 * `useQueue`, so mutations and the explicit queue refresh re-read this exact
 * task instead of leaving a stale card behind. A malformed id never produces a
 * request, and a missing or forbidden task stays a generic, actionable state.
 */
export function QueueFocusedTask({
  taskId,
  task,
  loading,
  error,
  onDismiss,
  isActing,
  onRetry,
}: {
  taskId: string;
  task: TaskResponse | null;
  loading: boolean;
  error: boolean;
  onDismiss: () => void;
  isActing: (task: TaskResponse) => boolean;
  onRetry: (task: TaskResponse) => void;
}) {
  const { t } = useTranslation();
  const valid = isValidTaskFocusId(taskId);
  const sectionRef = useRef<HTMLElement>(null);

  useEffect(() => {
    sectionRef.current?.focus();
  }, [taskId]);

  return (
    <section
      ref={sectionRef}
      tabIndex={-1}
      aria-labelledby="queue-focused-task-title"
      data-testid="queue-focused-task"
      className="rounded-md border bg-card outline-none"
    >
      <div className="flex flex-wrap items-start justify-between gap-2 border-b p-3">
        <div className="min-w-0 space-y-1">
          <h2 id="queue-focused-task-title" className="text-sm font-semibold">
            {t("processingQueue.focus.heading")}
          </h2>
          {task
            ? (
                <>
                  <p className="truncate text-sm" title={taskSubject(task) ?? undefined}>
                    {taskSubject(task) ?? t("processingQueue.unnamedTask")}
                  </p>
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <StatusBadge
                      tone={statusTone(task.status)}
                      data-testid="queue-focused-task-status"
                    >
                      {t(`processingQueue.statuses.${task.status}`)}
                    </StatusBadge>
                    <span>{queueStageLabel(t, task.stage)}</span>
                  </div>
                </>
              )
            : null}
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
          <X aria-hidden="true" />
          {t("processingQueue.focus.dismiss")}
        </Button>
      </div>

      {!valid
        ? (
            <p className="p-3 text-sm text-muted-foreground">{t("processingQueue.focus.invalid")}</p>
          )
        : error
          ? (
              <p className="p-3 text-sm text-muted-foreground" data-testid="queue-focused-task-error">
                {t("processingQueue.focus.unavailable")}
              </p>
            )
          : loading || !task
            ? <p className="p-3 text-sm text-muted-foreground">{t("common.loading")}</p>
            : (
                <QueueExpandedRow
                  task={task}
                  active={isActiveTask(task)}
                  isActing={isActing(task)}
                  onRetry={onRetry}
                />
              )}
    </section>
  );
}
