import { ListChecks, RotateCcw, Search, X } from "lucide-react";
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
 * expanded row, and a selection whose bulk actions state their scope and
 * consequences before they run.
 */
export function ProcessingQueuePage() {
  const { t } = useTranslation();
  const queue = useQueue();
  const busy = queue.selectionBusy;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("nav.processingQueue")}</h1>
        </div>
        <div className="flex flex-wrap items-center gap-2">
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
            <TabsTrigger key={view} value={view} disabled={busy}>
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
            disabled={busy}
          />
        </div>
      </form>

      <div className="flex flex-wrap items-center gap-2 rounded-md border px-2 py-1.5">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={queue.selectAllMatching}
        >
          <ListChecks aria-hidden="true" />
          {t("processingQueue.selectAllMatching")}
        </Button>
        <span className="text-sm text-muted-foreground" data-testid="queue-selection-count">
          {t("processingQueue.selectionCount", { count: queue.selectionCount })}
        </span>
        {queue.selection.snapshot
          ? (
              <span className="text-sm text-muted-foreground" data-testid="queue-selection-snapshot">
                {t("processingQueue.selectionSnapshot")}
              </span>
            )
          : null}
        {queue.selectionCount > 0
          ? (
              <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={queue.clearSelection}>
                <X aria-hidden="true" />
                {t("processingQueue.clearSelection")}
              </Button>
            )
          : null}
        {queue.bulkProgress
          ? (
              <span className="text-sm text-muted-foreground" data-testid="queue-bulk-progress">
                {t("processingQueue.bulkProgress", {
                  completed: queue.bulkProgress.completed,
                  total: queue.bulkProgress.total,
                })}
              </span>
            )
          : null}
        {queue.selectionCount > 0
          ? (
              <div className="ml-auto flex flex-wrap items-center gap-2" data-testid="queue-bulk-actions">
                {queue.bulkActions.map((action) => (
                  <Button
                    key={action}
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => queue.runBulkAction(action)}
                  >
                    {t(`processingQueue.bulkActions.${action}`)}
                  </Button>
                ))}
              </div>
            )
          : null}
      </div>

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
        isSelected={queue.isSelected}
        selectionDisabled={queue.selectionBusy}
        filtersDisabled={queue.selectionBusy}
        onToggleSelect={queue.toggleTaskSelected}
        onTogglePageSelection={queue.togglePageSelection}
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
            // Paging is frozen while a bulk action runs: the captured ids stay
            // valid and the operator stays on the window the action started in.
            <div
              inert={busy || undefined}
              aria-busy={busy || undefined}
              data-testid="queue-pagination"
            >
              <TablePagination
                pagination={queue.pagination}
                onPage={queue.setPage}
                onPageSize={queue.setPageSize}
              />
            </div>
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
