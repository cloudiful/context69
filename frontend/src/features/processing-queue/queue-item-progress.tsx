import { useTranslation } from "react-i18next";

import type { TaskItemResponse } from "@/services/api";

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
    <div className="flex items-center gap-2 whitespace-nowrap text-xs text-muted-foreground">
      {percent !== null
        ? (
            <div
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={percent}
              aria-label={view.stageLabel}
              className="h-1.5 w-20 shrink-0 overflow-hidden rounded-full bg-muted"
            >
              <div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} />
            </div>
          )
        : null}
      {countText ? <span>{countText}</span> : null}
      {view.remoteStatus
        ? <span>{t("processingQueue.itemProgress.remoteStatus", { status: view.remoteStatus })}</span>
        : null}
    </div>
  );
}
