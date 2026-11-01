import { ChevronLeft, ChevronRight, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

import {
  QUEUE_BULK_REPORT_ID,
  QUEUE_BULK_REPORT_PAGE_SIZE,
  type QueueBulkReport,
  type QueueBulkReportEntry,
} from "./queue-helpers";

function entryBadgeVariant(status: QueueBulkReportEntry["status"]): "secondary" | "outline" | "destructive" {
  switch (status) {
    case "accepted":
      return "secondary";
    case "skipped":
      return "outline";
    default:
      return "destructive";
  }
}

/**
 * The persistent result of the last bulk action: theme-readable rows, one page
 * of at most 50 entries, and the target the server reported for every accepted
 * row. A source reprocess links to the created child; an in-place recovery
 * links to the original task. The report stays until the next batch or an
 * explicit dismissal, so no per-item outcome disappears with a toast.
 */
export function QueueBulkReport({
  report,
  onDismiss,
  onOpenTask,
}: {
  report: QueueBulkReport;
  onDismiss: () => void;
  onOpenTask: (taskId: string) => void;
}) {
  const { t } = useTranslation();
  const [page, setPage] = useState(1);
  const previousReport = useRef(report);

  useEffect(() => {
    if (previousReport.current === report) {
      return;
    }
    previousReport.current = report;
    setPage(1);
  }, [report]);

  const pageCount = Math.max(1, Math.ceil(report.entries.length / QUEUE_BULK_REPORT_PAGE_SIZE));
  const currentPage = Math.min(page, pageCount);
  const start = (currentPage - 1) * QUEUE_BULK_REPORT_PAGE_SIZE;
  const pageEntries = report.entries.slice(start, start + QUEUE_BULK_REPORT_PAGE_SIZE);
  const { summary } = report;

  return (
    <section
      id={QUEUE_BULK_REPORT_ID}
      tabIndex={-1}
      aria-labelledby={`${QUEUE_BULK_REPORT_ID}-title`}
      data-testid="queue-bulk-report"
      className="space-y-2 rounded-md border bg-card p-3"
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="space-y-1">
          <h2 id={`${QUEUE_BULK_REPORT_ID}-title`} className="text-sm font-semibold">
            {t("processingQueue.bulkReport.title")}
          </h2>
          <p className="text-xs text-muted-foreground" data-testid="queue-bulk-report-summary">
            {t("processingQueue.bulkSummary", {
              succeeded: summary.succeeded,
              skipped: summary.skipped,
              failed: summary.failed,
            })}
          </p>
          {summary.partial > 0
            ? (
                <p className="text-xs text-muted-foreground">
                  {t("processingQueue.bulkPartial", { count: summary.partial })}
                </p>
              )
            : null}
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={onDismiss}>
          <X aria-hidden="true" />
          {t("processingQueue.bulkReport.dismiss")}
        </Button>
      </div>

      {pageEntries.length > 0
        ? (
            <ul className="divide-y rounded-md border">
              {pageEntries.map((entry) => {
                const subject = entry.subject ?? entry.taskId;
                const targetTaskId = entry.targetTaskId;
                const reason = entry.reason
                  ? t(`processingQueue.bulkReasons.${entry.reason}`, { defaultValue: entry.reason })
                  : null;
                return (
                  <li
                    key={entry.taskId}
                    data-testid="queue-bulk-report-row"
                    className="flex flex-wrap items-start justify-between gap-2 p-2"
                  >
                    <div className="min-w-0 flex-1 space-y-0.5">
                      <span className="block truncate text-sm" title={subject}>{subject}</span>
                      {reason
                        ? (
                            <span
                              className="block max-w-2xl truncate text-xs text-muted-foreground"
                              title={reason}
                            >
                              {reason}
                            </span>
                          )
                        : null}
                    </div>
                    <div className="flex items-center gap-2">
                      <Badge variant={entryBadgeVariant(entry.status)}>
                        {t(`processingQueue.bulkReport.statuses.${entry.status}`)}
                      </Badge>
                      {targetTaskId
                        ? (
                            <Button
                              type="button"
                              variant="outline"
                              size="sm"
                              aria-label={t("processingQueue.bulkReport.openTaskNamed", { name: subject })}
                              onClick={() => onOpenTask(targetTaskId)}
                            >
                              {targetTaskId === entry.taskId
                                ? t("processingQueue.bulkReport.viewTask")
                                : t("processingQueue.bulkReport.viewNewTask")}
                            </Button>
                          )
                        : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          )
        : null}

      {pageCount > 1
        ? (
            <nav
              className="flex items-center justify-end gap-2"
              aria-label={t("processingQueue.bulkReport.pagination")}
            >
              <Button
                type="button"
                variant="outline"
                size="icon-sm"
                aria-label={t("processingQueue.bulkReport.previousPage")}
                disabled={currentPage <= 1}
                onClick={() => setPage((value) => Math.max(1, value - 1))}
              >
                <ChevronLeft aria-hidden="true" />
              </Button>
              <span className="text-xs text-muted-foreground">
                {t("processingQueue.bulkReport.page", { page: currentPage, pages: pageCount })}
              </span>
              <Button
                type="button"
                variant="outline"
                size="icon-sm"
                aria-label={t("processingQueue.bulkReport.nextPage")}
                disabled={currentPage >= pageCount}
                onClick={() => setPage((value) => Math.min(pageCount, value + 1))}
              >
                <ChevronRight aria-hidden="true" />
              </Button>
            </nav>
          )
        : null}
    </section>
  );
}
