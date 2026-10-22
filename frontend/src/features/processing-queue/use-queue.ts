import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { apiClient } from "@/services/api";
import type {
  ClearTaskHistoryView,
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
  captureQueueRecoveryTargets,
  isRecoverableTask,
  isTerminalTask,
  runQueueBulkRecovery,
  summarizeQueueRecovery,
  summarizeResults,
  type QueueRecoveryAction,
} from "./queue-helpers";
import { applyTaskPatches, type QueueScope } from "./queue-updates";
import { useQueueLive } from "./use-queue-live";

const DEFAULT_PAGE_SIZE = 25;
/**
 * Deadline for one queue window read. A transport that never answers must not
 * leave the view loading forever: the read fails as a load error and a manual
 * refresh can start a new one.
 */
export const QUEUE_LIST_TIMEOUT_MS = 15_000;

export interface ConfirmRequest {
  title: string;
  message: string;
  acceptLabel: string;
  action: () => void;
}

export type BulkAction = "recover" | "cancel" | null;

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
  bulkAction: BulkAction;
  clearAction: ClearTaskHistoryView | null;
  confirm: ConfirmRequest | null;
  recoverableCount: number;
  activeCount: number;
  failedCount: number;
  cancelledCount: number;
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
  recoverTask: (task: TaskResponse) => void;
  cancelTask: (task: TaskResponse) => void;
  confirmTrashTask: (task: TaskResponse) => void;
  restoreTask: (task: TaskResponse) => void;
  confirmDeleteTask: (task: TaskResponse) => void;
  confirmRecoverAll: () => void;
  confirmCancelActive: () => void;
  confirmClearCompleted: () => void;
  confirmClearTrash: () => void;
  confirmAccept: () => void;
  confirmCancel: () => void;
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

/**
 * One server-state queue window plus its user actions.
 *
 * The list, its totals and every row live in ONE React Query cache entry keyed by
 * the complete scope. Live frames patch that cache in place; anything that could
 * change membership, order or totals queues a trailing authoritative GET. Local
 * state is limited to UI selections (expanded rows, in-flight actions).
 */
export function useQueue(): QueueState {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [scope, setScope] = useState<QueueScope>(initialScope);
  const [searchInput, setSearchInput] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [actionIds, setActionIds] = useState<string[]>([]);
  const [bulkAction, setBulkAction] = useState<BulkAction>(null);
  const [clearAction, setClearAction] = useState<ClearTaskHistoryView | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

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

  const recoverableCount = items.filter(isRecoverableTask).length;
  const activeCount = items.filter((task) => ACTIVE_STATUSES.includes(task.status)).length;
  const failedCount = items.filter((task) => task.status === "failed").length;
  const cancelledCount = items.filter((task) => task.status === "cancelled").length;

  const updateScope = useCallback((patch: Partial<QueueScope>, resetPage = true) => {
    setScope((current) => {
      const next = { ...current, ...patch };
      if (resetPage && patch.page === undefined) {
        next.page = 1;
      }
      return next;
    });
    setExpanded({});
  }, []);

  const submitSearch = useCallback((event?: { preventDefault: () => void }) => {
    event?.preventDefault();
    updateScope({ query: searchInput.trim() });
  }, [searchInput, updateScope]);

  // Tab changes move the typed list view at once. The backend view owns the
  // trash/succeeded predicate; a status only narrows it. Each view restores its
  // own ordering, and completed drops the stage/waiting filters it cannot show.
  const setView = useCallback((view: TaskListView) => {
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
    updateScope({ sort: { field, direction } });
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
    options: { clearStatusOnSuccess?: boolean } = {},
  ) => {
    beginAction(task.task_id);
    try {
      await mutate();
      if (options.clearStatusOnSuccess && scopeRef.current.status !== null) {
        setScope((current) => ({ ...current, status: null, page: 1 }));
      } else {
        reload();
      }
      toast.success(t(successKey), { description: task.task_id, duration: 2500 });
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t(failureKey),
        duration: 5000,
      });
    } finally {
      endAction(task.task_id);
    }
  }, [beginAction, endAction, reload, t]);

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
        );
      },
    });
  }, [actionIds, runTaskAction, t]);

  const submitRecovery = useCallback(async (taskId: string, action: QueueRecoveryAction) => {
    if (action === "rerun") {
      await apiClient.rerunTask(taskId);
      return;
    }
    await apiClient.retryTask(taskId);
  }, []);

  const recoverAll = useCallback(async () => {
    if (bulkAction || clearAction) {
      return;
    }
    // Capture the current page's recoverable rows once, before any request.
    const targets = captureQueueRecoveryTargets(items.filter(isRecoverableTask));
    if (targets.length === 0) {
      return;
    }
    setBulkAction("recover");
    try {
      const outcomes = await runQueueBulkRecovery(targets, submitRecovery);
      const { summary, resumedCount } = summarizeQueueRecovery(outcomes);
      if (resumedCount > 0 && scopeRef.current.status !== null) {
        setScope((current) => ({ ...current, status: null, page: 1 }));
      } else {
        reload();
      }
      toast[summary.failed === 0 ? "success" : "warning"](
        t("processingQueue.bulkSummary", {
          succeeded: summary.succeeded,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
        {
          description: resumedCount > 0
            ? t("processingQueue.bulkResumed", { count: resumedCount })
            : undefined,
          duration: 3500,
        },
      );
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("processingQueue.bulkRetryFailed"),
        duration: 5000,
      });
    } finally {
      setBulkAction(null);
    }
  }, [bulkAction, clearAction, items, reload, submitRecovery, t]);

  const cancelActive = useCallback(async () => {
    if (bulkAction || clearAction) {
      return;
    }
    // Capture the same page's active task ids; never an unscoped admin cancel.
    const taskIds = items
      .filter((task) => ACTIVE_STATUSES.includes(task.status))
      .map((task) => task.task_id);
    if (taskIds.length === 0) {
      return;
    }
    setBulkAction("cancel");
    try {
      const results = await Promise.allSettled(taskIds.map((taskId) => apiClient.cancelTask(taskId)));
      const summary = summarizeResults(results);
      reload();
      toast[summary.failed === 0 ? "success" : "warning"](
        t("processingQueue.bulkSummary", {
          succeeded: summary.succeeded,
          skipped: summary.skipped,
          failed: summary.failed,
        }),
        { duration: 3500 },
      );
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error ? error.message : t("processingQueue.bulkCancelFailed"),
        duration: 5000,
      });
    } finally {
      setBulkAction(null);
    }
  }, [bulkAction, clearAction, items, reload, t]);

  const confirmRecoverAll = useCallback(() => {
    if (bulkAction || clearAction || recoverableCount === 0) {
      return;
    }
    setConfirm({
      title: t("processingQueue.retryAll"),
      message: t("processingQueue.recoverAllConfirm", {
        failed: failedCount,
        cancelled: cancelledCount,
      }),
      acceptLabel: t("processingQueue.retryAllAction"),
      action: () => {
        setConfirm(null);
        void recoverAll();
      },
    });
  }, [bulkAction, cancelledCount, clearAction, failedCount, recoverAll, recoverableCount, t]);

  const confirmCancelActive = useCallback(() => {
    if (bulkAction || clearAction || activeCount === 0) {
      return;
    }
    setConfirm({
      title: t("processingQueue.cancelActive"),
      message: `${t("processingQueue.cancelActiveConfirm")} (${activeCount})`,
      acceptLabel: t("processingQueue.cancelActiveAction"),
      action: () => {
        setConfirm(null);
        void cancelActive();
      },
    });
  }, [activeCount, bulkAction, cancelActive, clearAction, t]);

  const clearHistory = useCallback(async (view: ClearTaskHistoryView) => {
    if (clearAction || bulkAction) {
      return;
    }
    setClearAction(view);
    try {
      const response = await apiClient.clearTaskHistory({ view });
      reload();
      toast.success(
        t(view === "completed"
          ? "processingQueue.clearCompletedAccepted"
          : "processingQueue.clearTrashAccepted"),
        { description: String(response.deleted_count), duration: 3000 },
      );
    } catch (error) {
      toast.error(t("common.error"), {
        description: error instanceof Error
          ? error.message
          : t(view === "completed"
            ? "processingQueue.clearCompletedFailed"
            : "processingQueue.clearTrashFailed"),
        duration: 5000,
      });
    } finally {
      setClearAction(null);
    }
  }, [bulkAction, clearAction, reload, t]);

  const confirmClear = useCallback((view: ClearTaskHistoryView) => {
    if (clearAction || bulkAction) {
      return;
    }
    setConfirm({
      title: t(view === "completed" ? "processingQueue.clearCompleted" : "processingQueue.clearTrash"),
      message: t(view === "completed"
        ? "processingQueue.clearCompletedConfirm"
        : "processingQueue.clearTrashConfirm"),
      acceptLabel: t(view === "completed"
        ? "processingQueue.clearCompletedAction"
        : "processingQueue.clearTrashAction"),
      action: () => {
        setConfirm(null);
        void clearHistory(view);
      },
    });
  }, [bulkAction, clearAction, clearHistory, t]);

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
    bulkAction,
    clearAction,
    confirm,
    recoverableCount,
    activeCount,
    failedCount,
    cancelledCount,
    searchInput,
    setSearchInput,
    submitSearch,
    setView,
    setStatus: (status) => updateScope({ status }),
    setKind: (kind) => updateScope({ kind }),
    setStage: (stage) => updateScope({ stage }),
    setWaitingReason: (waitingReason) => updateScope({ waitingReason }),
    setDependencyKey: (dependencyKey) => updateScope({ dependencyKey }),
    setPage: (page) => setScope((current) => ({ ...current, page })),
    setPageSize: (pageSize) => setScope((current) => ({ ...current, pageSize, page: 1 })),
    setSort,
    refresh: reload,
    toggleExpanded,
    isActing,
    recoverTask,
    cancelTask,
    confirmTrashTask,
    restoreTask,
    confirmDeleteTask,
    confirmRecoverAll,
    confirmCancelActive,
    confirmClearCompleted: () => confirmClear("completed"),
    confirmClearTrash: () => confirmClear("trash"),
    confirmAccept: () => {
      const request = confirm;
      setConfirm(null);
      request?.action();
    },
    confirmCancel: () => setConfirm(null),
  };
}
