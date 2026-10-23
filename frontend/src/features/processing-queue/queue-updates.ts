import type {
  SortDirection,
  TaskKind,
  TaskListView,
  TaskResponse,
  TaskSortBy,
  TaskStatus,
} from "@/services/api";

/** The complete server-side scope of one visible queue window. */
export interface QueueScope {
  view: TaskListView;
  page: number;
  pageSize: number;
  query: string;
  status: TaskStatus | null;
  kind: TaskKind | null;
  stage: string | null;
  waitingReason: string | null;
  dependencyKey: string | null;
  sort: { field: TaskSortBy; direction: SortDirection };
}

export const PATCH_WINDOW_MS = 100;
export const MAX_BUFFERED_TASKS = 100;

/**
 * Whether an incoming live frame requires an authoritative reload instead of an
 * in-place row patch. Membership, ordering, view predicate, every active filter
 * and free-text search are server decisions, so the client never guesses them.
 */
export function queueUpdateRequiresReload(
  current: TaskResponse,
  incoming: TaskResponse,
  scope: QueueScope,
): boolean {
  if (current.task_id !== incoming.task_id) {
    return true;
  }
  if ((current.deleted_at ?? null) !== (incoming.deleted_at ?? null)) {
    return true;
  }
  if (scope.view === "processing" && (current.status === "succeeded") !== (incoming.status === "succeeded")) {
    return true;
  }
  if (scope.view === "completed" && (current.status === "succeeded") !== (incoming.status === "succeeded")) {
    return true;
  }
  if (scope.status !== null && current.status !== incoming.status) {
    return true;
  }
  if (scope.kind !== null && current.kind !== incoming.kind) {
    return true;
  }
  if (scope.stage !== null && current.stage !== incoming.stage) {
    return true;
  }
  if (scope.waitingReason !== null && current.waiting_reason !== incoming.waiting_reason) {
    return true;
  }
  if (scope.dependencyKey !== null && current.dependency_key !== incoming.dependency_key) {
    return true;
  }
  if (current[scope.sort.field] !== incoming[scope.sort.field]) {
    return true;
  }
  // The server searches task and item payload fields the client does not see,
  // so an active search always resyncs rather than guessing membership.
  return scope.query.trim().length > 0;
}

export type PatchDecision =
  | { kind: "patch"; items: TaskResponse[] }
  | { kind: "stale" };

/**
 * Patches only rows already in the window. A live frame never inserts a row,
 * changes the total, or removes a row: those are authoritative-GET decisions.
 */
export function applyTaskPatches(
  items: readonly TaskResponse[],
  updates: ReadonlyMap<string, TaskResponse>,
  scope: QueueScope,
): PatchDecision {
  const windowIds = new Set(items.map((task) => task.task_id));
  let stale = false;
  for (const taskId of updates.keys()) {
    if (!windowIds.has(taskId)) {
      stale = true;
      break;
    }
  }
  if (stale) {
    return { kind: "stale" };
  }

  const patched = items.map((task) => {
    const update = updates.get(task.task_id);
    if (!update) {
      return task;
    }
    if (queueUpdateRequiresReload(task, update, scope)) {
      stale = true;
      return task;
    }
    return { ...task, ...update };
  });
  return stale ? { kind: "stale" } : { kind: "patch", items: patched };
}

export interface PatchBuffer {
  enqueue(task: TaskResponse): "buffered" | "full";
  flush(): Map<string, TaskResponse>;
  invalidate(): void;
  readonly size: number;
}

/**
 * Fixed-window frame coalescing: repeated frames for one task collapse into its
 * latest state, and a bounded buffer reports `full` so the caller can fall back
 * to an authoritative reload instead of dropping frames silently.
 */
export function createPatchBuffer(max = MAX_BUFFERED_TASKS): PatchBuffer {
  let pending = new Map<string, TaskResponse>();

  return {
    enqueue(task) {
      if (!pending.has(task.task_id) && pending.size >= max) {
        return "full";
      }
      pending.set(task.task_id, { ...(pending.get(task.task_id) ?? task), ...task });
      return "buffered";
    },
    flush() {
      const flushed = pending;
      pending = new Map();
      return flushed;
    },
    invalidate() {
      pending = new Map();
    },
    get size() {
      return pending.size;
    },
  };
}
