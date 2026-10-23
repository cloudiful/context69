import { ArrowDown, ArrowUp, Ban, ChevronRight, ChevronsUpDown, Play, RotateCcw, Trash2, Undo2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { StatusBadge, type StatusTone } from "@/components/common/status-badge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import type { SortDirection, TaskKind, TaskListView, TaskResponse, TaskSortBy, TaskStatus } from "@/services/api";
import { formatTimestamp } from "@/lib/format";
import { cn } from "@/lib/utils";

import { QueueExpandedRow } from "./queue-expanded-row";
import { QueueHeaderFilter, type QueueFilterOption } from "./queue-filters";
import {
  ACTIVE_STATUSES,
  QUEUE_DEPENDENCY_VALUES,
  QUEUE_STAGE_VALUES,
  QUEUE_WAITING_REASON_VALUES,
  TASK_KIND_VALUES,
  isRecoverableTask,
  libraryDependencyLabel,
  queueStageLabel,
  taskDocumentTitle,
  taskSubject,
  PROCESSING_STATUS_FILTERS,
} from "./queue-helpers";
import type { QueueScope } from "./queue-updates";

const SORTABLE_FIELDS: readonly TaskSortBy[] = ["kind", "group_path", "status", "stage", "created_at", "updated_at"];
const TIME_SORT_FIELDS: readonly TaskSortBy[] = ["created_at", "updated_at"];
const KNOWN_STAGES: ReadonlySet<string> = new Set(QUEUE_STAGE_VALUES);

export interface QueueTableProps {
  items: TaskResponse[];
  loading: boolean;
  scope: QueueScope;
  expanded: Record<string, boolean>;
  isActing: (task: TaskResponse) => boolean;
  onToggleExpand: (taskId: string) => void;
  onSort: (field: TaskSortBy | null, direction: SortDirection | null) => void;
  onStatusFilter: (status: TaskStatus | null) => void;
  onKindFilter: (kind: TaskKind | null) => void;
  onStageFilter: (stage: string | null) => void;
  onWaitingReasonFilter: (reason: string | null, group?: string | null) => void;
  onDependencyFilter: (dependency: string | null) => void;
  onRecover: (task: TaskResponse) => void;
  onCancel: (task: TaskResponse) => void;
  onTrash: (task: TaskResponse) => void;
  onRestore: (task: TaskResponse) => void;
  onDelete: (task: TaskResponse) => void;
}

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
    case "cancelled":
      return "neutral";
    default:
      return "neutral";
  }
}

/** One labeled primary action per display state, decided by task status. */
function primaryAction(task: TaskResponse): "cancel" | "retry" | "resume" | "trash" | null {
  if (ACTIVE_STATUSES.includes(task.status)) {
    return "cancel";
  }
  if (task.status === "failed") {
    return "retry";
  }
  if (task.status === "cancelled") {
    return "resume";
  }
  if (task.status === "succeeded") {
    return "trash";
  }
  return null;
}

export function QueueTable({
  items,
  loading,
  scope,
  expanded,
  isActing,
  onToggleExpand,
  onSort,
  onStatusFilter,
  onKindFilter,
  onStageFilter,
  onWaitingReasonFilter,
  onDependencyFilter,
  onRecover,
  onCancel,
  onTrash,
  onRestore,
  onDelete,
}: QueueTableProps) {
  const { t } = useTranslation();
  const completed = scope.view === "completed";
  const trash = scope.view === "trash";

  const allOption = (label: string): QueueFilterOption => ({ label, value: null });
  const options = (labels: readonly string[], prefix: string): QueueFilterOption[] => (
    labels.map((value) => ({ label: t(`${prefix}.${value}`), value }))
  );
  // "Dependency unavailable" is one filter with a nested dependency submenu:
  // selecting a child narrows both the reason and the library dependency.
  const waitingOptions: QueueFilterOption[] = [
    allOption(t("processingQueue.allWaitingReasons")),
    {
      label: t("processingQueue.waitingReasons.dependency"),
      value: "dependency",
      children: [
        allOption(t("processingQueue.allDependencies")),
        ...options(QUEUE_DEPENDENCY_VALUES, "processingQueue.dependencies"),
      ],
    },
    ...QUEUE_WAITING_REASON_VALUES.filter((value) => value !== "dependency")
      .map((value) => ({ label: t(`processingQueue.waitingReasons.${value}`), value })),
  ];

  function cycleSort(field: TaskSortBy) {
    const current = scope.sort;
    const preferred: SortDirection = TIME_SORT_FIELDS.includes(field) ? "desc" : "asc";
    if (current.field !== field) {
      onSort(field, preferred);
      return;
    }
    if (current.direction === preferred) {
      onSort(field, preferred === "desc" ? "asc" : "desc");
      return;
    }
    onSort(null, null);
  }

  function header(
    field: TaskSortBy,
    label: string,
    filter?: { label: string; options: QueueFilterOption[]; selected: string | null; onSelect: (value: string | null) => void },
  ) {
    const state = scope.sort.field === field ? scope.sort.direction : "none";
    const stateLabel = state === "asc"
      ? t("processingQueue.sortedAscending")
      : state === "desc"
        ? t("processingQueue.sortedDescending")
        : t("processingQueue.notSorted");
    const Icon = state === "asc" ? ArrowUp : state === "desc" ? ArrowDown : ChevronsUpDown;
    return (
      <div className="flex items-center gap-0.5">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="-ml-2 h-7 gap-1 px-2 font-medium"
          aria-label={`${t("processingQueue.sortBy", { column: label })}, ${stateLabel}`}
          data-sort={state}
          onClick={() => cycleSort(field)}
        >
          {label}
          <Icon aria-hidden="true" className={cn("size-3", state === "none" ? "opacity-40" : "opacity-100")} />
        </Button>
        {filter
          ? (
              <QueueHeaderFilter
                label={filter.label}
                options={filter.options}
                selected={filter.selected}
                active={filter.selected !== null}
                onSelect={filter.onSelect}
              />
            )
          : null}
      </div>
    );
  }

  const emptyLabel = completed
    ? t("processingQueue.tabs.noCompletedTasks")
    : trash
      ? t("processingQueue.tabs.noTrashedTasks")
      : t("processingQueue.noTasks");

  return (
    <div className="overflow-x-auto rounded-md border" data-testid="processing-queue-table">
      <Table className="min-w-[88rem]">
        <TableHeader>
          <TableRow>
            <TableHead className="w-10" />
            <TableHead>{t("processingQueue.task")}</TableHead>
            {header("kind", t("processingQueue.type"), {
              label: t("processingQueue.kindFilter"),
              options: [allOption(t("processingQueue.allKinds")), ...options(TASK_KIND_VALUES, "processingQueue.kinds")],
              selected: scope.kind,
              onSelect: (value) => onKindFilter(value as TaskKind | null),
            })}
            <TableHead>{header("group_path", t("processingQueue.group"))}</TableHead>
            {header("status", t("processingQueue.status"), scope.view === "processing"
                ? {
                    label: t("processingQueue.statusFilter"),
                    options: [allOption(t("processingQueue.allStatuses")), ...options(PROCESSING_STATUS_FILTERS, "processingQueue.statuses")],
                    selected: scope.status,
                    onSelect: (value) => onStatusFilter(value as TaskStatus | null),
                  }
                : undefined)}
            {completed
              ? (
                  <TableHead>
                    <QueueHeaderFilter
                      label={t("processingQueue.dependencyFilter")}
                      options={[allOption(t("processingQueue.allDependencies")), ...options(QUEUE_DEPENDENCY_VALUES, "processingQueue.dependencies")]}
                      selected={scope.dependencyKey}
                      active={scope.dependencyKey !== null}
                      onSelect={onDependencyFilter}
                    />
                  </TableHead>
                )
              : (
                  <>
                    <TableHead>{header("stage", t("processingQueue.stage"), {
                      label: t("processingQueue.stageFilter"),
                      options: [allOption(t("processingQueue.allStages")), ...options(QUEUE_STAGE_VALUES, "processingQueue.stages")],
                      selected: scope.stage,
                      onSelect: onStageFilter,
                    })}</TableHead>
                    <TableHead>
                      <div className="flex items-center gap-0.5">
                        <span className="px-1 font-medium">{t("processingQueue.waiting")}</span>
                        <QueueHeaderFilter
                          label={t("processingQueue.waitingReasonFilter")}
                          options={waitingOptions}
                          selected={scope.waitingReason}
                          active={scope.waitingReason !== null || scope.dependencyKey !== null}
                          onSelect={(value, group) => {
                            if (group != null) {
                              onWaitingReasonFilter(group, value);
                              return;
                            }
                            onWaitingReasonFilter(value);
                            if (value !== "dependency") {
                              onDependencyFilter(null);
                            }
                          }}
                        />
                      </div>
                    </TableHead>
                    <TableHead>{t("processingQueue.progress")}</TableHead>
                  </>
                )}
            <TableHead>{t("processingQueue.error")}</TableHead>
            <TableHead>{header("created_at", t("processingQueue.createdAt"))}</TableHead>
            <TableHead>{header("updated_at", completed ? t("processingQueue.completedAt") : t("processingQueue.updatedAt"))}</TableHead>
            <TableHead>{t("processingQueue.actions")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {items.length === 0
            ? (
                <TableRow>
                  <TableCell colSpan={completed ? 10 : 12} className="py-12 text-center text-sm text-muted-foreground">
                    {loading ? t("common.loading") : emptyLabel}
                  </TableCell>
                </TableRow>
              )
            : items.map((task) => {
                const isOpen = expanded[task.task_id] === true;
                const acting = isActing(task);
                const action = primaryAction(task);
                return (
                  <TaskRow
                    key={task.task_id}
                    task={task}
                    open={isOpen}
                    acting={acting}
                    action={action}
                    completed={completed}
                    trash={trash}
                    onToggleExpand={onToggleExpand}
                    onRecover={onRecover}
                    onCancel={onCancel}
                    onTrash={onTrash}
                    onRestore={onRestore}
                    onDelete={onDelete}
                  />
                );
              })}
        </TableBody>
      </Table>
    </div>
  );
}

function TaskRow({
  task,
  open,
  acting,
  action,
  completed,
  trash,
  onToggleExpand,
  onRecover,
  onCancel,
  onTrash,
  onRestore,
  onDelete,
}: {
  task: TaskResponse;
  open: boolean;
  acting: boolean;
  action: ReturnType<typeof primaryAction>;
  completed: boolean;
  trash: boolean;
  onToggleExpand: (taskId: string) => void;
  onRecover: (task: TaskResponse) => void;
  onCancel: (task: TaskResponse) => void;
  onTrash: (task: TaskResponse) => void;
  onRestore: (task: TaskResponse) => void;
  onDelete: (task: TaskResponse) => void;
}) {
  const { t } = useTranslation();
  const subject = taskSubject(task);
  const title = taskDocumentTitle(task);

  const waiting = task.waiting_reason
    ? `${t(`processingQueue.waitingReasons.${task.waiting_reason}`, { defaultValue: task.waiting_reason })}${
        task.dependency_key ? `: ${libraryDependencyLabel(t, task.dependency_key)}` : ""
      }`
    : "--";
  const errorStage = task.failure_stage && KNOWN_STAGES.has(task.failure_stage)
    ? queueStageLabel(t, task.failure_stage)
    : null;

  return (
    <>
      <TableRow data-testid="queue-task-row" data-state={open ? "open" : "closed"}>
        <TableCell>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-expanded={open}
            aria-label={open ? t("processingQueue.collapse") : t("processingQueue.expand")}
            onClick={() => onToggleExpand(task.task_id)}
          >
            <ChevronRight aria-hidden="true" className={cn("transition-transform", open && "rotate-90")} />
          </Button>
        </TableCell>
        <TableCell>
          <div className="min-w-0" data-testid="queue-task">
            <span className="block max-w-64 truncate" title={subject ?? undefined}>
              {subject ?? t("processingQueue.unnamedTask")}
            </span>
            {title && title !== subject
              ? <span className="block max-w-64 truncate text-xs text-muted-foreground" title={title}>{title}</span>
              : null}
          </div>
        </TableCell>
        <TableCell><Badge variant="secondary">{t(`processingQueue.kinds.${task.kind}`)}</Badge></TableCell>
        <TableCell>
          <span className="block max-w-48 truncate" title={task.group_path ?? undefined}>{task.group_path || "--"}</span>
        </TableCell>
        {trash
          ? null
          : (
              <TableCell>
                <StatusBadge tone={statusTone(task.status)}>{t(`processingQueue.statuses.${task.status}`)}</StatusBadge>
              </TableCell>
            )}
        {completed
          ? (
              <TableCell>
                <span className="block max-w-48 truncate text-muted-foreground" title={task.dependency_key ?? undefined}>
                  {libraryDependencyLabel(t, task.dependency_key ?? null)}
                </span>
              </TableCell>
            )
          : (
              <>
                <TableCell><span className="whitespace-nowrap text-muted-foreground">{queueStageLabel(t, task.stage)}</span></TableCell>
                <TableCell>
                  <span className="block max-w-48 truncate text-muted-foreground" title={waiting}>{waiting}</span>
                </TableCell>
                <TableCell>
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground" data-testid="queue-progress">
                    <span className="whitespace-nowrap font-mono">{task.progress.succeeded}/{task.progress.total}</span>
                    {task.progress.running
                      ? <span className="whitespace-nowrap">{t("processingQueue.statuses.running")} {task.progress.running}</span>
                      : null}
                    {task.progress.waiting
                      ? <span className="whitespace-nowrap">{t("processingQueue.statuses.waiting")} {task.progress.waiting}</span>
                      : null}
                    {task.progress.queued
                      ? <span className="whitespace-nowrap">{t("processingQueue.statuses.queued")} {task.progress.queued}</span>
                      : null}
                    {task.progress.failed
                      ? <span className="whitespace-nowrap text-destructive">{t("processingQueue.statuses.failed")} {task.progress.failed}</span>
                      : null}
                  </div>
                </TableCell>
              </>
            )}
        <TableCell>
          <span className="block max-w-80 truncate text-muted-foreground" title={task.error_summary ?? undefined} data-testid="queue-error">
            {task.error_summary
              ? (
                  <>
                    {errorStage}
                    {errorStage ? " " : ""}
                    {task.error_summary}
                  </>
                )
              : "--"}
          </span>
        </TableCell>
        <TableCell><span className="whitespace-nowrap text-muted-foreground">{formatTimestamp(task.created_at)}</span></TableCell>
        <TableCell><span className="whitespace-nowrap text-muted-foreground">{formatTimestamp(task.updated_at)}</span></TableCell>
        <TableCell>
          {trash
            ? (
                <div className="flex items-center gap-1">
                  <Button type="button" variant="ghost" size="icon-sm" disabled={acting} aria-label={t("processingQueue.restore")} title={t("processingQueue.restore")} onClick={() => onRestore(task)}>
                    <Undo2 aria-hidden="true" />
                  </Button>
                  <Button type="button" variant="ghost" size="icon-sm" disabled={acting} aria-label={t("processingQueue.deletePermanently")} title={t("processingQueue.deletePermanently")} onClick={() => onDelete(task)}>
                    <Trash2 aria-hidden="true" className="text-destructive" />
                  </Button>
                </div>
              )
            : (
                <div className="flex items-center gap-1">
                  {action === "retry" && isRecoverableTask(task)
                    ? (
                        <Button type="button" variant="ghost" size="sm" disabled={acting} title={t("processingQueue.retryTaskScope")} onClick={() => onRecover(task)}>
                          <RotateCcw aria-hidden="true" />
                          {t("processingQueue.retryTaskScope")}
                        </Button>
                      )
                    : null}
                  {action === "resume" && isRecoverableTask(task)
                    ? (
                        <Button type="button" variant="ghost" size="sm" disabled={acting} title={t("processingQueue.resumeHint")} onClick={() => onRecover(task)}>
                          <Play aria-hidden="true" />
                          {t("processingQueue.continue")}
                        </Button>
                      )
                    : null}
                  {action === "cancel"
                    ? (
                        <Button type="button" variant="ghost" size="icon-sm" disabled={acting} aria-label={t("processingQueue.cancel")} title={t("processingQueue.cancel")} onClick={() => onCancel(task)}>
                          <Ban aria-hidden="true" className="text-destructive" />
                        </Button>
                      )
                    : null}
                  {action === "trash" || task.status === "failed" || task.status === "cancelled"
                    ? (
                        <Button type="button" variant="ghost" size="icon-sm" disabled={acting} aria-label={t("processingQueue.trash")} title={t("processingQueue.trash")} onClick={() => onTrash(task)}>
                          <Trash2 aria-hidden="true" />
                        </Button>
                      )
                    : null}
                </div>
              )}
        </TableCell>
      </TableRow>
      {open
        ? (
            <TableRow>
              <TableCell colSpan={completed ? 10 : 12} className="p-0">
                <QueueExpandedRow
                  task={task}
                  active={ACTIVE_STATUSES.includes(task.status)}
                  isActing={acting}
                  onRetry={onRecover}
                />
              </TableCell>
            </TableRow>
          )
        : null}
    </>
  );
}
