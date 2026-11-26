import { useTable, type ColumnDef } from "@tanstack/react-table";
import { RotateCcw } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { dataTableFeatures, renderTable } from "@/components/common/data-table";
import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { TaskItemResponse, TaskItemStatus, TaskResponse } from "@/services/api";
import { formatTimestamp } from "@/lib/format";

import { QueueDiagnosePanel } from "./queue-diagnose-panel";
import { QueueItemProgress } from "./queue-item-progress";
import { taskItemStatusLabel, useTaskItems, itemStatuses } from "./use-task-items";

const ALL_ITEMS = "__all__";

function itemSubject(item: TaskItemResponse): string | null {
  return item.document_title?.trim()
    || item.file_name?.trim()
    || null;
}

function itemFileName(item: TaskItemResponse): string | null {
  return item.file_name?.trim() || null;
}

function itemStatusTone(status: TaskItemStatus): StatusTone {
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
    case "cancelled":
      return "neutral";
    default:
      return "neutral";
  }
}

/**
 * Expanded items for one task: cursor paging keeps the rows already loaded, the
 * status filter re-requests from the first page, and a failed refresh keeps the
 * last valid list visible while saying so. The task-level retry label states the
 * real scope, and the action is offered from the task status only, so a row with
 * `retryable: false` still lets the operator ask the server to decide.
 */
export function QueueExpandedRow({
  task,
  active,
  isActing,
  onRetry,
}: {
  task: TaskResponse;
  active: boolean;
  isActing: boolean;
  onRetry: (task: TaskResponse) => void;
}) {
  const { t } = useTranslation();
  const [diagnoseOpen, setDiagnoseOpen] = useState(false);
  const items = useTaskItems({ taskId: task.task_id, active, parentUpdatedAt: task.updated_at });
  const retryAvailable = task.status === "failed";
  const hasErrors = items.items.some((item) => Boolean(item.error_message));
  const columns = useMemo<ColumnDef<typeof dataTableFeatures, TaskItemResponse>[]>(() => [
    {
      id: "file",
      header: t("processingQueue.itemProgress.file"),
      cell: ({ row }) => {
        const item = row.original;
        const subject = itemSubject(item);
        const fileName = itemFileName(item);
        return (
          <div className="flex min-w-64 items-center gap-2">
            <span className="text-xs text-muted-foreground">#{item.ordinal}</span>
            <span className="max-w-md truncate font-medium" title={subject ?? item.item_id}>
              {subject ?? item.item_id.slice(0, 8)}
            </span>
            {fileName && fileName !== subject
              ? <span className="max-w-48 truncate text-xs text-muted-foreground" title={fileName}>{fileName}</span>
              : null}
          </div>
        );
      },
    },
    {
      accessorKey: "status",
      header: t("processingQueue.status"),
      cell: ({ row }) => (
        <div className="flex items-center gap-2">
          <StatusBadge tone={itemStatusTone(row.original.status)}>
            {taskItemStatusLabel(t, row.original.status)}
          </StatusBadge>
          {row.original.waiting_reason
            ? (
                <span className="text-xs text-muted-foreground">
                  {t(`processingQueue.waitingReasons.${row.original.waiting_reason}`, {
                    defaultValue: row.original.waiting_reason,
                  })}
                </span>
              )
            : null}
        </div>
      ),
    },
    {
      id: "progress",
      header: t("processingQueue.progress"),
      cell: ({ row }) => <QueueItemProgress progress={row.original.progress} />,
    },
    {
      id: "lastProgressAt",
      header: t("processingQueue.itemProgress.lastProgressAt"),
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {row.original.progress?.last_progress_at
            ? formatTimestamp(row.original.progress.last_progress_at)
            : "--"}
        </span>
      ),
    },
    {
      accessorKey: "attempt_count",
      header: t("processingQueue.itemProgress.attempts"),
      cell: ({ row }) => (
        <div className="flex items-center gap-3 text-xs text-muted-foreground">
          <span>{row.original.attempt_count}</span>
          {row.original.next_attempt_at
            ? <span>{t("processingQueue.diagnose.nextRetry", { time: formatTimestamp(row.original.next_attempt_at) })}</span>
            : null}
        </div>
      ),
    },
    ...(hasErrors ? [{
      accessorKey: "error_message",
      header: t("processingQueue.error"),
      cell: ({ row }) => (
        <p className="max-w-md whitespace-normal break-words text-xs text-destructive">
          {row.original.error_message}
        </p>
      ),
    } satisfies ColumnDef<typeof dataTableFeatures, TaskItemResponse>] : []),
    ...(retryAvailable ? [{
      id: "actions",
      header: t("processingQueue.actions"),
      cell: () => (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={isActing}
          title={t("processingQueue.retryTaskScope")}
          onClick={() => onRetry(task)}
        >
          <RotateCcw aria-hidden="true" />
          {t("processingQueue.retryTaskScope")}
        </Button>
      ),
    } satisfies ColumnDef<typeof dataTableFeatures, TaskItemResponse>] : []),
  ], [t, hasErrors, retryAvailable, isActing, onRetry, task]);
  const table = useTable({
    features: dataTableFeatures,
    data: items.items,
    columns,
    getRowId: (item) => item.item_id,
  });

  return (
    <div className="space-y-3 p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Select
            value={items.statusFilter ?? ALL_ITEMS}
            onValueChange={(value) => items.setStatusFilter(
              value === ALL_ITEMS ? null : value as TaskItemStatus,
            )}
          >
            <SelectTrigger size="sm" className="w-40" aria-label={t("processingQueue.itemStatusFilter")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL_ITEMS}>{t("processingQueue.allItemStatuses")}</SelectItem>
              {itemStatuses().map((status) => (
                <SelectItem key={status} value={status}>
                  {t(`processingQueue.statuses.${status}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span className="text-xs text-muted-foreground" data-testid="task-items-count">
            {t("processingQueue.itemsCount", { shown: items.items.length, total: task.progress.total })}
          </span>
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={() => setDiagnoseOpen((open) => !open)}>
          {diagnoseOpen ? t("processingQueue.diagnose.hide") : t("processingQueue.diagnose.action")}
        </Button>
      </div>

      {diagnoseOpen
        ? (
            <div className="space-y-1">
              <p className="text-xs text-muted-foreground">{t("processingQueue.diagnose.explanation")}</p>
              <QueueDiagnosePanel taskId={task.task_id} />
            </div>
          )
        : null}

      {items.refreshFailed
        ? <p className="text-xs text-muted-foreground">{t("processingQueue.itemProgress.refreshFailed")}</p>
        : null}

      {items.error
        ? (
            <div className="flex items-center gap-2 text-xs">
              <span className="text-destructive">{t("processingQueue.itemsLoadFailed")}</span>
              <Button type="button" variant="outline" size="sm" onClick={items.reload}>
                {t("processingQueue.retryLoadItems")}
              </Button>
            </div>
          )
        : null}

      {items.items.length === 0 && !items.loading && !items.error
        ? <p className="text-xs text-muted-foreground">{t("processingQueue.noItems")}</p>
        : (
            <div className="rounded-md border">
              {renderTable(table, t("processingQueue.noItems"), "task-item-row")}
            </div>
          )}

      {items.hasMore
        ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-testid="task-items-load-more"
              onClick={items.loadMore}
              disabled={items.loadingMore}
            >
              {t("processingQueue.loadMoreItems")}
            </Button>
          )
        : null}
    </div>
  );
}
