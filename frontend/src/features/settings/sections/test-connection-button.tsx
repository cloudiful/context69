import { useState } from "react";
import { Pause, Play, RefreshCw, Square, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Button } from "@/components/ui/button";
import { useSettings } from "@/features/settings/settings-context";

export function TestConnectionButton({
  testing,
  disabled,
  testId,
  label,
  onTest,
}: {
  testing: boolean;
  disabled?: boolean;
  testId: string;
  label: string;
  onTest: () => void;
}) {
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      data-testid={testId}
      disabled={testing || disabled}
      aria-busy={testing}
      onClick={onTest}
    >
      {testing ? <RefreshCw aria-hidden="true" className="animate-spin" /> : <Zap aria-hidden="true" />}
      {label}
    </Button>
  );
}

/**
 * Vector index rebuild trigger, its durable status and its controls.
 *
 * The status is the backend's durable projection, so it names the task every
 * control acts on: a control can only ever touch the rebuild the operator
 * observed. Resubmitting is never a control — a rebuild the operator wants to
 * stop is cancelled, not replaced.
 */
export function VectorRebuildControl() {
  const { t } = useTranslation();
  const { vector } = useSettings();
  const [cancelOpen, setCancelOpen] = useState(false);
  const status = vector.status;
  const state = status?.state ?? "idle";
  const active = state === "running" || state === "pausing" || state === "paused";

  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-testid="runtime-vector-rebuild"
          disabled={vector.running}
          aria-busy={vector.running}
          onClick={() => vector.setConfirmOpen(true)}
        >
          <RefreshCw aria-hidden="true" className={vector.running ? "animate-spin" : undefined} />
          <span>{t("settings.runtime.vectorRebuild")}</span>
        </Button>
        {vector.isAdmin && status?.can_pause ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="runtime-vector-rebuild-pause"
            disabled={vector.pending !== null}
            onClick={() => void vector.pause()}
          >
            <Pause aria-hidden="true" />
            <span>{t("settings.runtime.vectorRebuildPause")}</span>
          </Button>
        ) : null}
        {vector.isAdmin && status?.can_resume ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="runtime-vector-rebuild-resume"
            disabled={vector.pending !== null}
            onClick={() => void vector.resume()}
          >
            <Play aria-hidden="true" />
            <span>{t("settings.runtime.vectorRebuildResume")}</span>
          </Button>
        ) : null}
        {vector.isAdmin && status?.can_cancel ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="runtime-vector-rebuild-cancel"
            disabled={vector.pending !== null}
            onClick={() => setCancelOpen(true)}
          >
            <Square aria-hidden="true" />
            <span>{t("settings.runtime.vectorRebuildCancel")}</span>
          </Button>
        ) : null}
      </div>

      {vector.isAdmin ? (
        <>
          {state !== "idle" ? (
            <p className="text-sm text-muted-foreground" data-testid="runtime-vector-rebuild-state">
              {t(`settings.runtime.vectorRebuildStates.${state}`)}
            </p>
          ) : null}
          {active && status?.phase ? (
            <p className="text-xs text-muted-foreground" data-testid="runtime-vector-rebuild-phase">
              {t(`settings.runtime.vectorRebuildPhase.${status.phase}`)}
            </p>
          ) : null}
          {active && status && status.total_chunks > 0 ? (
            <p className="text-xs text-muted-foreground">
              {t("settings.runtime.vectorRebuildProgress", {
                processed: status.processed_chunks,
                total: status.total_chunks,
              })}
            </p>
          ) : null}
          {active && status && status.total_chunks === 0 ? (
            <>
              {status.documents_staged != null ? (
                <p className="text-xs text-muted-foreground" data-testid="runtime-vector-rebuild-documents">
                  {status.documents_total != null
                    ? t("settings.runtime.vectorRebuildDocumentsProgress", {
                        staged: status.documents_staged,
                        total: status.documents_total,
                      })
                    : t("settings.runtime.vectorRebuildDocumentsStaged", {
                        staged: status.documents_staged,
                      })}
                </p>
              ) : null}
              <p className="text-xs text-muted-foreground" data-testid="runtime-vector-rebuild-embedded">
                {t("settings.runtime.vectorRebuildEmbeddedChunks", {
                  processed: status.processed_chunks,
                })}
              </p>
            </>
          ) : null}
          {state === "pausing" || state === "paused" ? (
            <p className="text-xs text-muted-foreground">
              {t("settings.runtime.vectorRebuildPauseHelp")}
            </p>
          ) : null}
          {status?.queue_paused && state === "queued" ? (
            <p className="text-xs text-muted-foreground">
              {t("settings.runtime.vectorRebuildQueuedByQueue")}
            </p>
          ) : null}
          {state === "succeeded" && status ? (
            <p className="text-sm text-emerald-600 dark:text-emerald-400">
              {t("settings.runtime.vectorRebuildSucceeded", { count: status.processed_chunks })}
            </p>
          ) : null}
          {state === "failed" ? (
            <p className="text-sm text-destructive">
              {status?.error_message || t("settings.runtime.vectorRebuildFailed")}
            </p>
          ) : null}
          {state === "cancelled" ? (
            <p className="text-sm text-muted-foreground">
              {t("settings.runtime.vectorRebuildCancelledState")}
            </p>
          ) : null}
          {status && active && (status.active_dimensions != null || status.desired_dimensions != null) ? (
            <p className="text-xs text-muted-foreground" data-testid="runtime-vector-rebuild-dimensions">
              {t("settings.runtime.vectorRebuildDimensions", {
                active: status.active_dimensions ?? "—",
                desired: status.desired_dimensions ?? "—",
              })}
            </p>
          ) : null}
        </>
      ) : null}

      <ConfirmDialog
        open={cancelOpen}
        title={t("settings.runtime.vectorRebuildCancelTitle")}
        message={t("settings.runtime.vectorRebuildCancelConfirm")}
        acceptLabel={t("settings.runtime.vectorRebuildCancel")}
        onAccept={() => {
          setCancelOpen(false);
          void vector.cancel();
        }}
        onCancel={() => setCancelOpen(false)}
      />
    </div>
  );
}
