import { RefreshCw, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

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

/** Vector index rebuild trigger and its current task status. */
export function VectorRebuildControl() {
  const { t } = useTranslation();
  const { vector } = useSettings();

  return (
    <div className="grid gap-2">
      <div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          data-testid="runtime-vector-rebuild"
          disabled={vector.running}
          aria-busy={vector.running}
          onClick={() => vector.setConfirmOpen(true)}
        >
          {vector.running ? (
            <RefreshCw aria-hidden="true" className="animate-spin" />
          ) : (
            <RefreshCw aria-hidden="true" />
          )}
          <span>{t("settings.runtime.vectorRebuild")}</span>
        </Button>
      </div>
      {vector.status && vector.running ? (
        <p className="text-sm text-muted-foreground">
          {t(`processingQueue.statuses.${vector.status.status}`)}
        </p>
      ) : vector.status?.status === "succeeded" ? (
        <p className="text-sm text-emerald-600 dark:text-emerald-400">
          {t("settings.runtime.vectorRebuildSucceeded", { count: vector.status.progress.succeeded })}
        </p>
      ) : vector.status?.status === "failed" ? (
        <p className="text-sm text-destructive">
          {vector.status.error_summary || t("settings.runtime.vectorRebuildFailed")}
        </p>
      ) : null}
    </div>
  );
}
