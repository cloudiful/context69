import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";

import { apiClient } from "@/services/api";
import type { TaskItemResponse, TaskItemStatus } from "@/services/api";

/** Item page size for the expanded task panel. */
export const TASK_ITEMS_PAGE_SIZE = 100;
/** Periodic refresh only while the parent task is still active. */
export const TASK_ITEMS_REFRESH_MS = 3_000;

export function itemStatuses(): TaskItemStatus[] {
  return ["queued", "running", "waiting", "succeeded", "failed", "cancelled"];
}

const KNOWN_ITEM_STATUSES: ReadonlySet<string> = new Set(itemStatuses());

export function taskItemStatusLabel(
  t: (key: string) => string,
  status: string,
): string {
  return KNOWN_ITEM_STATUSES.has(status) ? t(`processingQueue.statuses.${status}`) : status;
}

export interface TaskItemsResult {
  items: TaskItemResponse[];
  nextCursor: string | null;
}

export interface UseTaskItemsOptions {
  taskId: string;
  /** Refreshing applies only while the task can still change. */
  active: boolean;
  /**
   * Revision of the parent task. A change, or the active-to-terminal
   * transition, means the loaded item pages may already be stale.
   */
  parentUpdatedAt?: string;
}

/**
 * Cursor-paged items for one expanded task.
 *
 * The request key carries the task id, the item status filter and how many pages
 * the operator has loaded, so a filter change or a different task cannot apply an
 * older response. `loadMore` extends that extent and keeps the rows already
 * shown, and the periodic refresh reloads exactly the loaded extent instead of
 * collapsing back to page one. A failed refresh keeps the last valid rows and
 * cursor; only an explicit retry clears the failure notice.
 */
export function useTaskItems({ taskId, active, parentUpdatedAt }: UseTaskItemsOptions) {
  const [statusFilter, setStatusFilter] = useState<TaskItemStatus | null>(null);
  // Loaded page count is a request extent, not part of the query identity: a
  // failed load-more must keep the rows and cursor already shown, and only a
  // successful fetch may replace them.
  const extentRef = useRef(1);

  // A new task always starts from the first page with no item filter.
  const previousTaskRef = useRef(taskId);
  useEffect(() => {
    if (previousTaskRef.current === taskId) {
      return;
    }
    previousTaskRef.current = taskId;
    extentRef.current = 1;
    setStatusFilter(null);
  }, [taskId]);

  const applyStatusFilter = useCallback((next: TaskItemStatus | null) => {
    setStatusFilter((current) => {
      if (current === next) {
        return current;
      }
      extentRef.current = 1;
      return next;
    });
  }, []);

  const query = useQuery({
    queryKey: ["tasks", "items", taskId, statusFilter ?? "all"],
    queryFn: async ({ signal }): Promise<TaskItemsResult> => {
      const collected: TaskItemResponse[] = [];
      let cursor: string | null = null;
      for (let page = 0; page < extentRef.current; page += 1) {
        const response = await apiClient.getTaskItems(taskId, {
          limit: TASK_ITEMS_PAGE_SIZE,
          status: statusFilter,
          ...(cursor === null ? {} : { cursor }),
        }, { signal });
        collected.push(...response.items);
        cursor = response.next_cursor ?? null;
        if (cursor === null) {
          break;
        }
      }
      return { items: collected, nextCursor: cursor };
    },
    retry: false,
    placeholderData: keepPreviousData,
    refetchInterval: active ? TASK_ITEMS_REFRESH_MS : false,
    refetchIntervalInBackground: false,
    staleTime: 0,
  });

  // A parent revision or the final transition asks for one authoritative reload
  // of the loaded extent. React Query would cancel and restart an in-flight
  // request for a plain `refetch`, so a request that is already running absorbs
  // the demand and a single trailing reload runs once it settles.
  const pendingRefresh = useRef(false);
  const refreshInFlight = useRef(false);

  const startRefresh = useCallback(() => {
    pendingRefresh.current = false;
    refreshInFlight.current = true;
    void query.refetch({ cancelRefetch: false }).finally(() => {
      refreshInFlight.current = false;
      // Nothing else observes the flag; the flush effect below picks it up.
    });
  }, [query]);

  const requestRefresh = useCallback(() => {
    if (query.isFetching || refreshInFlight.current) {
      pendingRefresh.current = true;
      return;
    }
    startRefresh();
  }, [query.isFetching, startRefresh]);

  // One parent update is one refresh signal: a changed revision and the
  // active-to-terminal transition arrive together and must not queue twice.
  const previousParent = useRef({ revision: parentUpdatedAt, active });
  useEffect(() => {
    const previous = previousParent.current;
    previousParent.current = { revision: parentUpdatedAt, active };
    if (previous.revision !== parentUpdatedAt || (previous.active && !active)) {
      requestRefresh();
    }
  }, [active, parentUpdatedAt, requestRefresh]);

  useEffect(() => {
    if (pendingRefresh.current && !refreshInFlight.current && !query.isFetching) {
      startRefresh();
    }
  }, [query.isFetching, startRefresh]);

  const items = query.data?.items ?? [];
  const hasMore = (query.data?.nextCursor ?? null) !== null;

  const loadMore = useCallback(() => {
    if (!hasMore || query.isFetching) {
      return;
    }
    extentRef.current += 1;
    void query.refetch();
  }, [hasMore, query, query.isFetching]);

  return {
    items,
    hasMore,
    statusFilter,
    setStatusFilter: applyStatusFilter,
    loading: query.isLoading,
    loadingMore: query.isFetching && items.length > 0,
    // A refresh failure keeps the last valid list; the notice is informational.
    refreshFailed: query.isError && items.length > 0,
    error: query.isError && items.length === 0 ? query.error : null,
    reload: () => {
      void query.refetch();
    },
    loadMore,
  };
}
