import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type {
  SortDirection,
  TaskKind,
  TaskListView,
  TaskPageResponse,
  TaskResponse,
  TaskSortBy,
  TaskStatus,
} from "@/services/api";

import {
  ACTIVE_STATUSES,
  DEFAULT_QUEUE_SORT_BY_VIEW,
  QUEUE_BULK_REPORT_ID,
  QueuePartialFailureError,
  QueueTaskSkippedError,
  buildQueueBulkReport,
  captureQueueBulkTargets,
  isActiveStatus,
  isActiveTask,
  isQueueBulkActionApplicable,
  isRecoverableTask,
  isSourceRerunnableTask,
  isTerminalTask,
  queueBulkActionOptions,
  runQueueBatchRerun,
  runQueueBatchRetry,
  runQueueBulkTasks,
  summarizeQueueBulkOutcomes,
  taskSubject,
  type QueueBulkAction,
  type QueueBulkActionOption,
  type QueueBulkReport,
  type QueueBulkTarget,
} from "./queue-helpers";
import { applyTaskPatches, type QueueScope } from "./queue-updates";
import { useQueueLive } from "./use-queue-live";
import { TASK_ITEMS_REFRESH_MS } from "./use-task-items";

const DEFAULT_PAGE_SIZE = 25;
/**
 * Deadline for one queue window read. A transport that never answers must not
 * leave the view loading forever: the read fails as a load error and a manual
 * refresh can start a new one.
 */
export const QUEUE_LIST_TIMEOUT_MS = 15_000;

const TASK_FOCUS_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** The focused-task detail query key, shared by focus reads, mutations and refresh. */
export function taskFocusQueryKey(taskId: string) {
  return ["tasks", "detail", taskId] as const;
}

/** A malformed link is rejected before any request is built from it. */
export function isValidTaskFocusId(value: string | null | undefined): value is string {
  return typeof value === "string" && TASK_FOCUS_ID.test(value);
}

/** A focused task still in progress is re-read on the same cadence as its items. */
export const FOCUSED_TASK_REFRESH_MS = TASK_ITEMS_REFRESH_MS;

export interface ConfirmRequest {
  title: string;
  message: string;
  acceptLabel: string;
  action: () => void;
}

/**
 * The selected tasks: the ids the operator picked, the status each id had when
 * it was picked, and whether the ids came from a captured matching set.
 */
export interface QueueSelection {
  ids: string[];
  statuses: Record<string, TaskStatus>;
  snapshot: boolean;
}

export interface QueueBulkProgress {
  completed: number;
  total: number;
}

/** The page owns navigation, so the hook borrows only the task-open behavior. */
export interface UseQueueOptions {
  openTask?: (taskId: string) => void;
  /** The `?task=<uuid>` value the page reads from its route. */
  focusedTaskId?: string | null;
  /**
   * Whether the admin queue-admission controls are available.
   *
   * Default false: a caller that does not opt in never issues the admin queue
   * read and never sees a pause control.
   */
  enableQueueControls?: boolean;
}

const EMPTY_SELECTION: QueueSelection = { ids: [], statuses: {}, snapshot: false };

/** The cache key of the durable global queue-admission state. */
const queueStateQueryKey = ["tasks", "queue-state"] as const;

class QueueListTimeoutError extends Error {
  constructor() {
    super("Queue list request timed out");
    this.name = "QueueListTimeoutError";
  }
}

/**
 * Runs one list read under a request-local deadline. The timeout aborts the
 * transport and rejects as a real load failure, and the returned promise settles
 * even when the transport ignores the abort, so the query can never stay pending.
 * Every timer and listener is released on success, failure or cancellation.
 */
async function readListWithinDeadline(
  read: (signal: AbortSignal) => Promise<TaskPageResponse>,
  lifecycle: AbortSignal,
): Promise<TaskPageResponse> {
  const controller = new AbortController();
  let settled = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let rejectDeadline: ((reason: unknown) => void) | null = null;

  const deadline = new Promise<never>((_resolve, reject) => {
    rejectDeadline = reject;
  });

  const fail = (reason: unknown) => {
    if (settled) {
      return;
    }
    settled = true;
    rejectDeadline?.(reason);
  };

  const abortFromLifecycle = () => {
    controller.abort();
    fail(new DOMException("The queue read was cancelled", "AbortError"));
  };

  if (lifecycle.aborted) {
    abortFromLifecycle();
  } else {
    lifecycle.addEventListener("abort", abortFromLifecycle, { once: true });
  }

  timer = setTimeout(() => {
    const error = new QueueListTimeoutError();
    controller.abort(error);
    fail(error);
  }, QUEUE_LIST_TIMEOUT_MS);

  const pending = read(controller.signal);
  // The deadline may settle first while a transport ignores the abort; keep that
  // late rejection from surfacing as an unhandled one.
  pending.catch(() => undefined);

  try {
    return await Promise.race([pending, deadline]);
  } finally {
    settled = true;
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    lifecycle.removeEventListener("abort", abortFromLifecycle);
  }
}

export interface QueueState {
  scope: QueueScope;
  items: TaskResponse[];
  pagination: TaskPageResponse["pagination"] | null;
  loading: boolean;
  error: unknown;
  live: { streaming: boolean; connected: boolean; fallback: boolean };
  expanded: Record<string, boolean>;
  actionIds: string[];
  selection: QueueSelection;
  selectionCount: number;
  bulkAction: QueueBulkAction | null;
  bulkProgress: QueueBulkProgress | null;
  bulkActions: QueueBulkAction[];
  /** The available actions with their counted eligible labels. */
  bulkActionOptions: QueueBulkActionOption[];
  /** The last bulk action's persistent per-item report. */
  bulkReport: QueueBulkReport | null;
  dismissBulkReport: () => void;
  /** The authoritative detail behind the focused task card, if one is valid. */
  focusedTask: TaskResponse | null;
  focusedTaskLoading: boolean;
  focusedTaskError: boolean;
  /** A bulk action is running or a snapshot is being captured. */
  selectionBusy: boolean;
  capturingSelection: boolean;
  confirm: ConfirmRequest | null;
  searchInput: string;
  setSearchInput: (value: string) => void;
  submitSearch: (event?: { preventDefault: () => void }) => void;
  setView: (view: TaskListView) => void;
  setStatus: (status: TaskStatus | null) => void;
  setKind: (kind: TaskKind | null) => void;
  setStage: (stage: string | null) => void;
  setWaitingReason: (reason: string | null) => void;
  setDependencyKey: (dependency: string | null) => void;
  setPage: (page: number) => void;
  setPageSize: (size: number) => void;
  setSort: (field: TaskSortBy | null, direction: SortDirection | null) => void;
  refresh: () => void;
  toggleExpanded: (taskId: string) => void;
  isActing: (task: TaskResponse) => boolean;
  isSelected: (taskId: string) => boolean;
  toggleTaskSelected: (task: TaskResponse) => void;
  togglePageSelection: () => void;
  clearSelection: () => void;
  selectAllMatching: () => void;
  runBulkAction: (action: QueueBulkAction) => void;
  recoverTask: (task: TaskResponse) => void;
  rerunTask: (task: TaskResponse) => void;
  cancelTask: (task: TaskResponse) => void;
  confirmTrashTask: (task: TaskResponse) => void;
  restoreTask: (task: TaskResponse) => void;
  confirmDeleteTask: (task: TaskResponse) => void;
  confirmAccept: () => void;
  confirmCancel: () => void;
  /** Whether the admin queue controls are available to this caller. */
  queueControls: boolean;
  /** The durable global admission state; null until the first admin read. */
  queuePaused: boolean | null;
  /** Whether the durable admission state has been read successfully. */
  queueStateKnown: boolean;
  /** Whether the durable admission state read is in flight. */
  queueStateLoading: boolean;
  /** The failed durable admission state read, when the state is unknown. */
  queueStateError: unknown;
  /** Retry the durable admission state read after a failure. */
  refreshQueueState: () => void;
  /** Whether a queue-state write is in flight. */
  queueControlBusy: boolean;
  toggleQueuePaused: () => void;
}

function initialScope(): QueueScope {
  return {
    view: "processing",
    page: 1,
    pageSize: DEFAULT_PAGE_SIZE,
    query: "",
    status: null,
    kind: null,
    stage: null,
    waitingReason: null,
    dependencyKey: null,
    sort: { ...DEFAULT_QUEUE_SORT_BY_VIEW.processing },
  };
}

/** A conflict answer: the task is no longer in the state the action needs. */
function isConflict(error: unknown): boolean {
  return (error as { status?: unknown } | null)?.status === 409;
}

/**
 * One server-state queue window plus its user actions.
 *
 * The list, its totals and every row live in ONE React Query cache entry keyed by
 * the complete scope. Live frames patch that cache in place; anything that could
 * change membership, order or totals queues a trailing authoritative GET. Local
 * state is limited to UI selections (expanded rows, in-flight actions, the
 * selected task ids).
 */
export function useQueue(options: UseQueueOptions = {}): QueueState {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const enableQueueControls = options.enableQueueControls === true;
  const [queueControlBusy, setQueueControlBusy] = useState(false);
  const [scope, setScope] = useState<QueueScope>(initialScope);
  const [searchInput, setSearchInput] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [actionIds, setActionIds] = useState<string[]>([]);
  const [selection, setSelection] = useState<QueueSelection>(EMPTY_SELECTION);
  const [bulkAction, setBulkAction] = useState<QueueBulkAction | null>(null);
  const [bulkProgress, setBulkProgress] = useState<QueueBulkProgress | null>(null);
  const [bulkReport, setBulkReport] = useState<QueueBulkReport | null>(null);
  // Synchronous guard: two clicks in one tick both see the old state, so the
  // ref is what actually prevents a duplicate batch submission.
  const bulkRunningRef = useRef(false);
  const [capturingSelection, setCapturingSelection] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

  const openTaskRef = useRef(options.openTask);
  openTaskRef.current = options.openTask;

  const focusedTaskId = isValidTaskFocusId(options.focusedTaskId) ? options.focusedTaskId : null;
  const focusedTaskIdRef = useRef<string | null>(focusedTaskId);
  focusedTaskIdRef.current = focusedTaskId;

  // The focused task is one extra authoritative read, keyed by its own id so a
  // navigation cannot apply an older response. While the task can still change
  // it is polled on the same cadence as the item panel; a terminal task stops
  // the interval, so a settled focus never keeps requesting.
  const focusedTaskQuery = useQuery({
    queryKey: taskFocusQueryKey(focusedTaskId ?? ""),
    queryFn: ({ signal }) => {
      if (!focusedTaskId) {
        throw new Error("No focused task");
      }
      return apiClient.getTask(focusedTaskId, { signal });
    },
    enabled: focusedTaskId !== null,
    retry: false,
    refetchInterval: (query) => {
      const task = query.state.data;
      return task && isActiveTask(task) ? FOCUSED_TASK_REFRESH_MS : false;
    },
    refetchIntervalInBackground: false,
  });

  /**
   * The durable global admission state of the queue.
   *
   * The read is admin-scoped, so the query stays disabled until the page opts
   * in with an admin viewer: a non-admin never issues it. The hook is always
   * created, so the hook order never depends on the viewer's role.
   */
  const queueStateQuery = useQuery({
    queryKey: queueStateQueryKey,
    queryFn: ({ signal }) => apiClient.getTaskQueueState({ signal }),
    enabled: enableQueueControls,
    retry: false,
  });

  const toggleQueuePaused = useCallback(() => {
    // The durable state is the authority of the toggle direction. Until it was
    // read successfully the button stays unavailable: treating an unknown
    // state as "not paused" would send a blind pause that can never resume an
    // already paused deployment.
    if (!enableQueueControls || queueControlBusy || queueStateQuery.data === undefined) {
      return;
    }
    const next = !queueStateQuery.data.paused;
    setQueueControlBusy(true);
    void (async () => {
      try {
        const response = await apiClient.updateTaskQueueState(next);
        queryClient.setQueryData(queueStateQueryKey, response);
        toast.success(
          t(next ? "processingQueue.queuePaused" : "processingQueue.queueResumed"),
          { duration: 2500 },
        );
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error && error.message
            ? error.message
            : t("processingQueue.queueControlFailed"),
          duration: 5000,
        });
      } finally {
        setQueueControlBusy(false);
      }
    })();
  }, [enableQueueControls, queryClient, queueControlBusy, queueStateQuery.data, t]);

  /**
   * Re-read one focused detail query by id. A mutation passes the id it
   * changed; the explicit queue refresh passes the current focus. Inactive ids
   * are only marked stale, so an unfocused task never issues a request.
   */
  const refetchFocusedTask = useCallback((taskId: string | null = focusedTaskIdRef.current) => {
    if (!isValidTaskFocusId(taskId)) {
      return;
    }
    void queryClient.invalidateQueries({ queryKey: taskFocusQueryKey(taskId), exact: true });
  }, [queryClient]);

  const scopeRef = useRef(scope);
  scopeRef.current = scope;

  const queryKey = useMemo(() => ["tasks", "list", scope] as const, [scope]);
  const queryKeyRef = useRef(queryKey);
  queryKeyRef.current = queryKey;

  // Ordering metadata for live frames, not a second copy of server state: every
  // successful list read of the *current* window advances the landing counter,
  // so a frame received before that read cannot overwrite its result afterwards.
  const authoritativeLandingRef = useRef(0);
  const getAuthoritativeLanding = useCallback(() => authoritativeLandingRef.current, []);

  const listQuery = useQuery({
    queryKey,
    queryFn: async ({ signal }) => {
      // The scope object this request belongs to. A response for a window that
      // has since been left is abandoned: it must not reorder live frames that
      // belong to the window the operator is actually looking at.
      const requestScope = scope;
      const result = await readListWithinDeadline(
        (requestSignal) => apiClient.listTasks({
          page: scope.page,
          pageSize: scope.pageSize,
          query: scope.query,
          kind: scope.kind,
          status: scope.status,
          view: scope.view,
          stage: scope.stage,
          waitingReason: scope.waitingReason,
          dependencyKey: scope.dependencyKey,
          sortBy: scope.sort.field,
          sortDirection: scope.sort.direction,
        }, { signal: requestSignal }),
        signal,
      );
      if (!signal.aborted && scopeRef.current === requestScope) {
        authoritativeLandingRef.current += 1;
      }
      return result;
    },
    retry: false,
    placeholderData: keepPreviousData,
  });

  const data = listQuery.data;
  const items = data?.items ?? [];
  const pagination = data?.pagination ?? null;
  const fetchingRef = useRef(listQuery.isFetching);
  fetchingRef.current = listQuery.isFetching;

  // Single-flight authoritative reload: triggers that arrive mid-flight coalesce
  // into one trailing request for the newest scope, and no trigger ever aborts a
  // request that is already in flight.
  const reloadInFlightRef = useRef<Promise<void> | null>(null);
  const reloadPendingRef = useRef(false);
  const reload = useCallback((): void => {
    // Never abort a request that is already running: mark one trailing reload
    // instead, and the effect below starts it once the fetch settles.
    if (reloadInFlightRef.current || fetchingRef.current) {
      reloadPendingRef.current = true;
      return;
    }
    const run = async () => {
      do {
        reloadPendingRef.current = false;
        await queryClient.refetchQueries({
          queryKey: queryKeyRef.current,
          exact: true,
        });
      } while (reloadPendingRef.current);
    };
    reloadInFlightRef.current = run().finally(() => {
      reloadInFlightRef.current = null;
    });
  }, [queryClient]);

  useEffect(() => {
    if (listQuery.isFetching || reloadInFlightRef.current || !reloadPendingRef.current) {
      return;
    }
    reloadPendingRef.current = false;
    reload();
  }, [listQuery.isFetching, reload]);

  const applyPatches = useCallback((updates: ReadonlyMap<string, TaskResponse>): "patched" | "stale" => {
    const current = queryClient.getQueryData<TaskPageResponse>(queryKeyRef.current);
    if (!current) {
      return "stale";
    }
    const decision = applyTaskPatches(current.items, updates, scopeRef.current);
    if (decision.kind === "stale") {
      return "stale";
    }
    queryClient.setQueryData<TaskPageResponse>(queryKeyRef.current, {
      ...current,
      items: decision.items,
    });
    return "patched";
  }, [queryClient]);

  const live = useQueueLive({
    enabled: scope.view === "processing",
    scope,
    applyPatches,
    reload,
    isFetching: () => fetchingRef.current,
    getAuthoritativeLanding,
  });

  // Expansion is UI state. It survives same-window reloads and progress frames,
  // and is dropped only when an authoritative window no longer contains the row
  // (a real view/page/filter change re-keys the query and reloads it).
  const authoritativeIds = data?.items.map((task) => task.task_id).join(",") ?? null;
  const authoritativeIdsRef = useRef<string | null>(null);
  useEffect(() => {
    if (authoritativeIds === null) {
      return;
    }
    if (authoritativeIdsRef.current === null) {
      authoritativeIdsRef.current = authoritativeIds;
      return;
    }
    if (authoritativeIdsRef.current === authoritativeIds) {
      return;
    }
    authoritativeIdsRef.current = authoritativeIds;
    const visible = new Set(authoritativeIds.split(",").filter(Boolean));
    setExpanded((current) => {
      const retained = Object.entries(current).filter(([taskId]) => visible.has(taskId));
      return retained.length === Object.keys(current).length
        ? current
        : Object.fromEntries(retained);
    });
  }, [authoritativeIds]);

  // The status of a selected task: the live status of the row on the current
  // page wins over the status captured with the selection, so an eligibility
  // decision never uses a state the list has already corrected.
  const selectionStatuses = useMemo(() => {
    const merged: Record<string, TaskStatus> = { ...selection.statuses };
    for (const task of items) {
      if (task.task_id in merged) {
        merged[task.task_id] = task.status;
      }
    }
    return merged;
  }, [items, selection.statuses]);

  const selectedStatuses = useMemo(
    () => selection.ids
      .map((taskId) => selectionStatuses[taskId])
      .filter((status): status is TaskStatus => status !== undefined),
    [selection.ids, selectionStatuses],
  );

  const bulkActionOptions = useMemo(
    () => queueBulkActionOptions(t, selectedStatuses, scope.view),
    [scope.view, selectedStatuses, t],
  );

  const bulkActions = useMemo(
    () => bulkActionOptions.map((option) => option.action),
    [bulkActionOptions],
  );

  // Readable subjects captured with the selection, so the persistent report can
  // name a row whose page has since been left. Off-page ids stay without one.
  const subjects = useMemo(() => {
    const names: Record<string, string> = {};
    for (const task of items) {
      const subject = taskSubject(task);
      if (subject) {
        names[task.task_id] = subject;
      }
    }
    return names;
  }, [items]);

  // While a bulk action runs or a snapshot is being captured, the window and
  // the selection are frozen: a filter or view change would hide the tasks P1
  // keeps selected for a retry, and a snapshot answer for another scope would
  // replace the selection with ids the new view cannot act on. The controls are
  // disabled and these guards keep any other path from doing it anyway.
  const selectionBusyRef = useRef(bulkAction !== null || capturingSelection);
  selectionBusyRef.current = bulkAction !== null || capturingSelection;

  const updateScope = useCallback((
    patch: Partial<QueueScope>,
    options: { resetPage?: boolean; clearSelection?: boolean } = {},
  ) => {
    if (selectionBusyRef.current) {
      return;
    }
    const { resetPage = true, clearSelection: dropSelection = true } = options;
    setScope((current) => {
      const next = { ...current, ...patch };
      if (resetPage && patch.page === undefined) {
        next.page = 1;
      }
      return next;
    });
    setExpanded({});
    // A changed view or filter can hide a destructive selection, so the
    // selection is dropped. Paging and sorting keep it.
    if (dropSelection) {
      setSelection(EMPTY_SELECTION);
    }
  }, []);

  const submitSearch = useCallback((event?: { preventDefault: () => void }) => {
    event?.preventDefault();
    updateScope({ query: searchInput.trim() });
  }, [searchInput, updateScope]);

  // Tab changes move the typed list view at once. The backend view owns the
  // trash/succeeded predicate; a status only narrows it. Each view restores its
  // own ordering, and completed drops the stage/waiting filters it cannot show.
  const setView = useCallback((view: TaskListView) => {
    if (selectionBusyRef.current) {
      return;
    }
    setScope((current) => ({
      ...current,
      view,
      page: 1,
      status: null,
      stage: view === "completed" ? null : current.stage,
      waitingReason: view === "completed" ? null : current.waitingReason,
      dependencyKey: view === "completed" ? null : current.dependencyKey,
      sort: { ...DEFAULT_QUEUE_SORT_BY_VIEW[view] },
    }));
    setExpanded({});
    setSelection(EMPTY_SELECTION);
  }, []);

  const setSort = useCallback((field: TaskSortBy | null, direction: SortDirection | null) => {
    if (field === null || direction === null) {
      setScope((current) => ({
        ...current,
        page: 1,
        sort: { ...DEFAULT_QUEUE_SORT_BY_VIEW[current.view] },
      }));
      return;
    }
    // Sorting reorders the window, it does not change what is selected.
    updateScope({ sort: { field, direction } }, { clearSelection: false });
  }, [updateScope]);

  const isActing = useCallback(
    (task: TaskResponse) => actionIds.includes(task.task_id),
    [actionIds],
  );

  const beginAction = useCallback((taskId: string) => {
    setActionIds((current) => (current.includes(taskId) ? current : [...current, taskId]));
  }, []);
  const endAction = useCallback((taskId: string) => {
    setActionIds((current) => current.filter((id) => id !== taskId));
  }, []);

  const runTaskAction = useCallback(async (
    task: TaskResponse,
    mutate: () => Promise<unknown>,
    successKey: string,
    failureKey: string,
    options: { clearStatusOnSuccess?: boolean; viewTarget?: string | null } = {},
  ) => {
    beginAction(task.task_id);
    try {
      await mutate();
      // The focused detail is authoritative per task: a successful mutation
      // refreshes exactly this id alongside the list, so the card cannot keep
      // offering the action the server has just accepted.
      refetchFocusedTask(task.task_id);
      if (options.clearStatusOnSuccess && scopeRef.current.status !== null) {
        setScope((current) => ({ ...current, status: null, page: 1 }));
      } else {
        reload();
      }
      // The accepted action target is the original task unless the mutation
      // created one: a source reprocess links to the child, a deletion has
      // nothing left to open.
      const target = options.viewTarget === undefined ? task.task_id : options.viewTarget;
      toast.success(t(successKey), {
        description: task.task_id,
        duration: 2500,
        action: target
          ? {
              label: t("processingQueue.viewTask"),
              onClick: () => openTaskRef.current?.(target),
            }
          : undefined,
      });
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t(failureKey),
        duration: 5000,
      });
    } finally {
      endAction(task.task_id);
    }
  }, [beginAction, endAction, refetchFocusedTask, reload, t]);

  const recoverTask = useCallback((task: TaskResponse) => {
    if (!isRecoverableTask(task) || actionIds.includes(task.task_id)) {
      return;
    }
    const resumed = task.status === "cancelled";
    void runTaskAction(
      task,
      () => (resumed ? apiClient.rerunTask(task.task_id) : apiClient.retryTask(task.task_id)),
      resumed ? "processingQueue.resumeAccepted" : "processingQueue.retryAccepted",
      resumed ? "processingQueue.resumeFailed" : "processingQueue.retryFailed",
      // A resumed task leaves its terminal status, so a narrowing status filter
      // would hide the row the user just recovered.
      { clearStatusOnSuccess: resumed },
    );
  }, [actionIds, runTaskAction]);

  /**
   * Reprocess a succeeded task from its current source. The server creates a
   * child task that reads the source again; the parent keeps its history. The
   * response is not discarded: its child id is what "View task" opens.
   */
  const rerunTask = useCallback((task: TaskResponse) => {
    if (!isSourceRerunnableTask(task) || actionIds.includes(task.task_id)) {
      return;
    }
    beginAction(task.task_id);
    void (async () => {
      try {
        const response = await apiClient.rerunTask(task.task_id);
        const childId = response.task.task_id;
        // The parent keeps its identity and its detail; the created child is
        // focused only when the operator asks for it.
        refetchFocusedTask(task.task_id);
        reload();
        toast.success(t("processingQueue.rerunAccepted"), {
          description: childId,
          duration: 6000,
          action: childId
            ? {
                label: t("processingQueue.viewTask"),
                onClick: () => openTaskRef.current?.(childId),
              }
            : undefined,
        });
      } catch (error) {
        toast.error(t("common.error"), {
          description: error instanceof Error ? error.message : t("processingQueue.rerunFailed"),
          duration: 5000,
        });
      } finally {
        endAction(task.task_id);
      }
    })();
  }, [actionIds, beginAction, endAction, refetchFocusedTask, reload, t]);

  const cancelTask = useCallback((task: TaskResponse) => {
    if (!ACTIVE_STATUSES.includes(task.status) || actionIds.includes(task.task_id)) {
      return;
    }
    void runTaskAction(
      task,
      () => apiClient.cancelTask(task.task_id),
      "processingQueue.cancelAccepted",
      "processingQueue.cancelFailed",
    );
  }, [actionIds, runTaskAction]);

  const restoreTask = useCallback((task: TaskResponse) => {
    if (actionIds.includes(task.task_id)) {
      return;
    }
    void runTaskAction(
      task,
      () => apiClient.restoreTask(task.task_id),
      "processingQueue.restoreAccepted",
      "processingQueue.restoreFailed",
    );
  }, [actionIds, runTaskAction]);

  const confirmTrashTask = useCallback((task: TaskResponse) => {
    // Only a terminal task can be trashed: an active one must be cancelled first.
    if (!isTerminalTask(task) || actionIds.includes(task.task_id)) {
      return;
    }
    setConfirm({
      title: t("processingQueue.trash"),
      message: t("processingQueue.trashConfirm"),
      acceptLabel: t("processingQueue.trashAction"),
      action: () => {
        setConfirm(null);
        void runTaskAction(
          task,
          () => apiClient.trashTask(task.task_id),
          "processingQueue.trashAccepted",
          "processingQueue.trashFailed",
        );
      },
    });
  }, [actionIds, runTaskAction, t]);

  const confirmDeleteTask = useCallback((task: TaskResponse) => {
    if (actionIds.includes(task.task_id)) {
      return;
    }
    setConfirm({
      title: t("processingQueue.deletePermanently"),
      message: t("processingQueue.deletePermanentlyConfirm"),
      acceptLabel: t("processingQueue.deletePermanentlyAction"),
      action: () => {
        setConfirm(null);
        void runTaskAction(
          task,
          () => apiClient.deleteTask(task.task_id),
          "processingQueue.deleteAccepted",
          "processingQueue.deleteFailed",
          { viewTarget: null },
        );
      },
    });
  }, [actionIds, runTaskAction, t]);

  const isSelected = useCallback(
    (taskId: string) => selection.ids.includes(taskId),
    [selection.ids],
  );

  const toggleTaskSelected = useCallback((task: TaskResponse) => {
    if (selectionBusyRef.current) {
      return;
    }
    setSelection((current) => {
      const statuses = { ...current.statuses };
      if (current.ids.includes(task.task_id)) {
        delete statuses[task.task_id];
        return {
          ...current,
          ids: current.ids.filter((taskId) => taskId !== task.task_id),
          statuses,
        };
      }
      statuses[task.task_id] = task.status;
      return { ...current, ids: [...current.ids, task.task_id], statuses };
    });
  }, []);

  const togglePageSelection = useCallback(() => {
    if (selectionBusyRef.current) {
      return;
    }
    setSelection((current) => {
      const pageIds = items.map((task) => task.task_id);
      const allSelected = pageIds.length > 0
        && pageIds.every((taskId) => current.ids.includes(taskId));
      const statuses = { ...current.statuses };
      if (allSelected) {
        for (const taskId of pageIds) {
          delete statuses[taskId];
        }
        return {
          ...current,
          ids: current.ids.filter((taskId) => !pageIds.includes(taskId)),
          statuses,
        };
      }
      for (const task of items) {
        statuses[task.task_id] = task.status;
      }
      const missing = pageIds.filter((taskId) => !current.ids.includes(taskId));
      return { ...current, ids: [...current.ids, ...missing], statuses };
    });
  }, [items]);

  const clearSelection = useCallback(() => {
    if (selectionBusyRef.current) {
      return;
    }
    setSelection(EMPTY_SELECTION);
  }, []);

  /**
   * Capture every task that matches the current filters, independent of the
   * page the operator looks at. The captured ids are frozen here: a later bulk
   * action acts on exactly these ids, so a task that arrives meanwhile is never
   * part of the action.
   */
  const selectAllMatching = useCallback(() => {
    if (selectionBusyRef.current) {
      return;
    }
    // The scope the capture belongs to. A response for a window that has since
    // been left is dropped instead of replacing the selection with ids the
    // current view cannot act on.
    const current = scopeRef.current;
    setCapturingSelection(true);
    void (async () => {
      try {
        const response = await apiClient.listTaskSelection({
          query: current.query,
          kind: current.kind,
          status: current.status,
          view: current.view,
          stage: current.stage,
          waitingReason: current.waitingReason,
          dependencyKey: current.dependencyKey,
        });
        if (scopeRef.current !== current) {
          toast.error(t("processingQueue.selectionFailed"), {
            description: t("processingQueue.selectionScopeChanged"),
            duration: 5000,
          });
          return;
        }
        const statuses: Record<string, TaskStatus> = {};
        for (const item of response.items) {
          statuses[item.task_id] = item.status;
        }
        setSelection({ ids: response.items.map((item) => item.task_id), statuses, snapshot: true });
        toast.success(t("processingQueue.selectionCaptured", { count: response.total }), {
          duration: 3000,
        });
      } catch (error) {
        toast.error(t("processingQueue.selectionFailed"), {
          description: error instanceof Error ? error.message : undefined,
          duration: 5000,
        });
      } finally {
        setCapturingSelection(false);
      }
    })();
  }, [t]);

  /**
   * Move a selected task to the trash. An active task is cancelled first and
   * only trashed after the cancellation succeeded; a task that turns terminal
   * in between is re-read and trashed when the re-read confirms it.
   */
  const submitTrashTarget = useCallback(async (target: QueueBulkTarget) => {
    let cancelled = false;
    if (isActiveStatus(target.status)) {
      try {
        await apiClient.cancelTask(target.taskId);
        cancelled = true;
      } catch (error) {
        if (!isConflict(error)) {
          throw error;
        }
        const current = await apiClient.getTask(target.taskId);
        if (isActiveStatus(current.status)) {
          throw error;
        }
      }
    }
    try {
      return await apiClient.trashTask(target.taskId);
    } catch (error) {
      if (cancelled) {
        throw new QueuePartialFailureError(t("processingQueue.bulkPartialFailure"));
      }
      throw error;
    }
  }, [t]);

  const submitBulkTarget = useCallback(async (target: QueueBulkTarget) => {
    switch (target.action) {
      case "cancel":
        return apiClient.cancelTask(target.taskId);
      case "trash":
        return submitTrashTarget(target);
      case "restore":
        return apiClient.restoreTask(target.taskId);
      case "delete":
        return apiClient.deleteTask(target.taskId);
      default:
        throw new QueueTaskSkippedError(t("processingQueue.bulkSkippedNotCancelled"));
    }
  }, [submitTrashTarget, t]);

  const runBulkAction = useCallback(async (action: QueueBulkAction) => {
    if (bulkAction || bulkRunningRef.current) {
      return;
    }
    const targets = captureQueueBulkTargets(selection.ids, selectionStatuses, action, subjects)
      .filter((target) => isQueueBulkActionApplicable(action, target.status));
    if (targets.length === 0) {
      return;
    }
    bulkRunningRef.current = true;
    setBulkAction(action);
    setBulkReport(null);
    setBulkProgress({ completed: 0, total: targets.length });
    try {
      const onProgress = (completed: number, total: number) => setBulkProgress({ completed, total });
      const outcomes = action === "retry"
        // One bounded batch API call per chunk, never a per-task fan-out: the
        // server retries failed and resumes cancelled items in place.
        ? await runQueueBatchRetry(
            targets,
            (taskIds) => apiClient.batchRetryTasks([...taskIds]),
            onProgress,
          )
        : action === "rerun"
          // The same bounded batch call, but for the succeeded parents that
          // own a rerunnable source; every other id is reported per item.
          ? await runQueueBatchRerun(
              targets,
              (taskIds) => apiClient.batchRerunTasks([...taskIds]),
              onProgress,
            )
          : await runQueueBulkTasks(
              targets,
              submitBulkTarget,
              onProgress,
            );
      const summary = summarizeQueueBulkOutcomes(outcomes);
      const report = buildQueueBulkReport(action, outcomes);
      setBulkReport(report);
      // The tasks that were handled leave the selection; the skipped and failed
      // ones stay selected so they can be seen and acted on again.
      const handled = new Set(
        outcomes
          .filter((outcome) => outcome.status === "succeeded")
          .map((outcome) => outcome.taskId),
      );
      setSelection((current) => {
        const statuses = Object.fromEntries(
          Object.entries(current.statuses).filter(([taskId]) => !handled.has(taskId)),
        );
        const ids = current.ids.filter((taskId) => !handled.has(taskId));
        return {
          ...current,
          ids,
          statuses,
          snapshot: current.snapshot && ids.length > 0,
        };
      });
      // Only the focused id's own detail is refreshed when the batch accepted
      // it; other accepted ids have no active detail observer and issue nothing.
      const focusedTaskId = focusedTaskIdRef.current;
      if (focusedTaskId && handled.has(focusedTaskId)) {
        refetchFocusedTask(focusedTaskId);
      }
      reload();
      // The toast stays brief and offers the persistent report instead of
      // joining per-id reasons into one long description.
      toast[summary.failed === 0 && summary.skipped === 0 ? "success" : "warning"](
        t("processingQueue.bulkSummary", {
          succeeded: summary.succeeded,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
        {
          action: {
            label: t("processingQueue.bulkReport.details"),
            onClick: () => document.getElementById(QUEUE_BULK_REPORT_ID)?.focus(),
          },
          duration: 3500,
        },
      );
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("processingQueue.bulkFailed"),
        duration: 5000,
      });
    } finally {
      bulkRunningRef.current = false;
      setBulkAction(null);
      setBulkProgress(null);
    }
  }, [bulkAction, reload, selection.ids, selectionStatuses, subjects, submitBulkTarget, refetchFocusedTask, t]);

  const requestBulkAction = useCallback((action: QueueBulkAction) => {
    if (bulkAction || selection.ids.length === 0) {
      return;
    }
    // The confirmation states the eligible count, not a total that includes
    // selected rows this action cannot touch.
    const count = captureQueueBulkTargets(selection.ids, selectionStatuses, action)
      .filter((target) => isQueueBulkActionApplicable(action, target.status))
      .length;
    if (count === 0) {
      return;
    }
    if (action === "retry" || action === "rerun" || action === "restore") {
      void runBulkAction(action);
      return;
    }
    setConfirm({
      title: t(`processingQueue.bulkTitles.${action}`),
      message: t(`processingQueue.bulkConfirms.${action}`, { count }),
      acceptLabel: t(`processingQueue.bulkActions.${action}`),
      action: () => {
        setConfirm(null);
        void runBulkAction(action);
      },
    });
  }, [bulkAction, runBulkAction, selection.ids, selectionStatuses, t]);

  const toggleExpanded = useCallback((taskId: string) => {
    setExpanded((current) => {
      const next = { ...current };
      if (next[taskId]) {
        delete next[taskId];
      } else {
        next[taskId] = true;
      }
      return next;
    });
  }, []);

  return {
    scope,
    items,
    pagination,
    loading: listQuery.isLoading,
    error: listQuery.error,
    live,
    expanded,
    actionIds,
    selection,
    selectionCount: selection.ids.length,
    bulkAction,
    bulkProgress,
    bulkActions,
    bulkActionOptions,
    bulkReport,
    dismissBulkReport: () => setBulkReport(null),
    focusedTask: focusedTaskId !== null ? focusedTaskQuery.data ?? null : null,
    focusedTaskLoading: focusedTaskId !== null && focusedTaskQuery.isLoading,
    focusedTaskError: focusedTaskId !== null && focusedTaskQuery.isError,
    selectionBusy: bulkAction !== null || capturingSelection,
    capturingSelection,
    confirm,
    searchInput,
    setSearchInput,
    submitSearch,
    setView,
    setStatus: (status) => updateScope({ status }),
    setKind: (kind) => updateScope({ kind }),
    setStage: (stage) => updateScope({ stage }),
    setWaitingReason: (waitingReason) => updateScope({ waitingReason }),
    setDependencyKey: (dependencyKey) => updateScope({ dependencyKey }),
    setPage: (page) => {
      if (selectionBusyRef.current) {
        return;
      }
      setScope((current) => ({ ...current, page }));
    },
    setPageSize: (pageSize) => {
      if (selectionBusyRef.current) {
        return;
      }
      setScope((current) => ({ ...current, pageSize, page: 1 }));
    },
    setSort,
    refresh: () => {
      reload();
      refetchFocusedTask();
      // Refresh is the operator's explicit reread: the durable admission state
      // is part of the queue and must be refetched instead of staying stale.
      if (enableQueueControls) {
        void queryClient.invalidateQueries({ queryKey: queueStateQueryKey });
      }
    },
    toggleExpanded,
    isActing,
    isSelected,
    toggleTaskSelected,
    togglePageSelection,
    clearSelection,
    selectAllMatching,
    runBulkAction: requestBulkAction,
    recoverTask,
    rerunTask,
    cancelTask,
    confirmTrashTask,
    restoreTask,
    confirmDeleteTask,
    confirmAccept: () => {
      const request = confirm;
      setConfirm(null);
      request?.action();
    },
    confirmCancel: () => setConfirm(null),
    queueControls: enableQueueControls,
    queuePaused: queueStateQuery.data?.paused ?? null,
    queueStateKnown: queueStateQuery.isSuccess && queueStateQuery.data !== undefined,
    queueStateLoading: queueStateQuery.isLoading,
    queueStateError: queueStateQuery.isError ? queueStateQuery.error : null,
    refreshQueueState: () => {
      void queueStateQuery.refetch();
    },
    queueControlBusy,
    toggleQueuePaused,
  };
}
