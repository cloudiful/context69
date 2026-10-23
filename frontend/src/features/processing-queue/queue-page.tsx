import { RotateCcw, Search, Trash2, XCircle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { TablePagination } from "@/components/common/table-pagination";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { TaskListView } from "@/services/api";

import { QueueTable } from "./queue-table";
import { useQueue } from "./use-queue";

const VIEWS: TaskListView[] = ["processing", "completed", "trash"];

/**
 * The processing queue workbench: three typed views over one server window,
 * header filters on the columns they narrow, cursor-paged items inside an
 * expanded row, and recovery actions whose scope and consequences are stated on
 * the control and confirmed before they run.
 */
export function ProcessingQueuePage() {
  const { t } = useTranslation();
  const queue = useQueue();
  const busy = queue.bulkAction !== null || queue.clearAction !== null;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("nav.processingQueue")}</h1>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {queue.scope.view !== "trash" && queue.recoverableCount > 0
            ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  onClick={queue.confirmRecoverAll}
                >
                  <RotateCcw aria-hidden="true" />
                  {`${t("processingQueue.retryAll")} (${queue.recoverableCount})`}
                </Button>
              )
            : null}
          {queue.scope.view !== "trash" && queue.activeCount > 0
            ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  onClick={queue.confirmCancelActive}
                >
                  <XCircle aria-hidden="true" />
                  {`${t("processingQueue.cancelActive")} (${queue.activeCount})`}
                </Button>
              )
            : null}
          {queue.scope.view === "completed"
            ? (
                <Button type="button" variant="outline" size="sm" disabled={busy} onClick={queue.confirmClearCompleted}>
                  <Trash2 aria-hidden="true" />
                  {t("processingQueue.clearCompleted")}
                </Button>
              )
            : null}
          {queue.scope.view === "trash"
            ? (
                <Button type="button" variant="outline" size="sm" disabled={busy} onClick={queue.confirmClearTrash}>
                  <Trash2 aria-hidden="true" />
                  {t("processingQueue.clearTrash")}
                </Button>
              )
            : null}
          <Button type="button" variant="outline" size="sm" onClick={queue.refresh}>
            <RotateCcw aria-hidden="true" />
            {t("processingQueue.refresh")}
          </Button>
        </div>
      </div>

      <Tabs
        value={queue.scope.view}
        onValueChange={(value) => queue.setView(value as TaskListView)}
      >
        <TabsList variant="line">
          {VIEWS.map((view) => (
            <TabsTrigger key={view} value={view}>
              {t(`processingQueue.tabs.${view}`)}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>

      <form
        className="flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          queue.submitSearch();
        }}
      >
        <div className="relative w-full max-w-sm">
          <Search aria-hidden="true" className="pointer-events-none absolute top-1/2 left-2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={queue.searchInput}
            onChange={(event) => queue.setSearchInput(event.target.value)}
            placeholder={t("processingQueue.searchPlaceholder")}
            title={t("processingQueue.searchHint")}
            aria-label={t("processingQueue.searchPlaceholder")}
            className="pl-8"
          />
        </div>
      </form>

      {queue.error
        ? (
            <Alert variant="destructive">
              <AlertTitle>{t("processingQueue.loadFailed")}</AlertTitle>
              <AlertDescription>
                {queue.error instanceof Error ? queue.error.message : null}
              </AlertDescription>
            </Alert>
          )
        : null}

      <QueueTable
        items={queue.items}
        loading={queue.loading}
        scope={queue.scope}
        expanded={queue.expanded}
        isActing={queue.isActing}
        onToggleExpand={queue.toggleExpanded}
        onSort={queue.setSort}
        onStatusFilter={queue.setStatus}
        onKindFilter={queue.setKind}
        onStageFilter={queue.setStage}
        onWaitingReasonFilter={(reason, group) => {
          if (group != null) {
            queue.setWaitingReason(group);
            queue.setDependencyKey(reason);
            return;
          }
          queue.setWaitingReason(reason);
        }}
        onDependencyFilter={queue.setDependencyKey}
        onRecover={queue.recoverTask}
        onCancel={queue.cancelTask}
        onTrash={queue.confirmTrashTask}
        onRestore={queue.restoreTask}
        onDelete={queue.confirmDeleteTask}
      />

      {queue.pagination
        ? (
            <TablePagination
              pagination={queue.pagination}
              onPage={queue.setPage}
              onPageSize={queue.setPageSize}
            />
          )
        : null}

      <ConfirmDialog
        open={queue.confirm !== null}
        title={queue.confirm?.title ?? ""}
        message={queue.confirm?.message ?? ""}
        acceptLabel={queue.confirm?.acceptLabel}
        onAccept={queue.confirmAccept}
        onCancel={queue.confirmCancel}
      />
    </div>
  );
}
