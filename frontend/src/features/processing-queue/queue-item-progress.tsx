import { useTranslation } from "react-i18next";

import type { TaskItemResponse } from "@/services/api";
import { formatTimestamp } from "@/lib/format";

import { itemProgressView, progressCountText, progressPercent } from "./queue-helpers";

/**
 * Stage-local progress for one task item. Counters come straight from the
 * server record; a missing total stays "unknown" rather than rendering a
 * guessed percentage, and the remote converter's own status is the only claim
 * made about a stage it does not measure.
 */
export function QueueItemProgress({ progress }: { progress?: TaskItemResponse["progress"] | null }) {
  const { t } = useTranslation();
  const view = itemProgressView(t, progress);
  if (!view) {
    return null;
  }

  const count = view.chunks ?? view.batches;
  const percent = progressPercent(count);
  const countText = progressCountText(t, view.chunks, "chunks")
    ?? progressCountText(t, view.batches, "batches");

  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap items-baseline justify-between gap-2 text-xs">
        <span className="font-medium">{view.stageLabel}</span>
        {view.remoteStatus
          ? <span className="text-muted-foreground">{t("processingQueue.itemProgress.remoteStatus", { status: view.remoteStatus })}</span>
          : null}
      </div>
      {percent !== null
        ? (
            <div
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent}
              aria-label={view.stageLabel}
              className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
            >
              <div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} />
            </div>
          )
        : null}
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
        <span>{countText ?? (view.isComplete ? t("processingQueue.itemProgress.complete") : t("processingQueue.itemProgress.countUnknown"))}</span>
        {view.lastProgressAt
          ? <span>{t("processingQueue.itemProgress.lastObserved", { time: formatTimestamp(view.lastProgressAt) })}</span>
          : null}
      </div>
      {view.showsPageProgressUnavailable
        ? <p className="text-xs text-muted-foreground">{t("processingQueue.itemProgress.pageProgressUnavailable")}</p>
        : null}
    </div>
  );
}
