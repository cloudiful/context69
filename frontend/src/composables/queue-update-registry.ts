import type { Ref } from "vue";

import type { SortDirection, TaskKind, TaskListView, TaskResponse, TaskSortBy, TaskStatus } from "../services/api";

const UPDATE_WINDOW_MS = 100;
const MAX_BUFFERED_TASKS = 100;

export interface QueueUpdateQuery {
  query: string;
  view: TaskListView;
  status: TaskStatus | null;
  kind: TaskKind | null;
  stage: string | null;
  waitingReason: string | null;
  dependencyKey: string | null;
  sort: { field: TaskSortBy; direction: SortDirection } | null;
}

export interface QueueUpdateRegistryOptions {
  items: Ref<TaskResponse[]>;
  isLoading: () => boolean;
  onLoadingUpdate: () => void;
  onStalePage: () => void;
}

export function queueUpdateRequiresReload(
  current: TaskResponse,
  incoming: TaskResponse,
  query: QueueUpdateQuery,
): boolean {
  if (current.task_id !== incoming.task_id) return true;
  if ((current.deleted_at ?? null) !== (incoming.deleted_at ?? null)) return true;

  if (query.view === "processing" && (current.status === "succeeded") !== (incoming.status === "succeeded")) return true;
  if (query.view === "completed" && (current.status === "succeeded") !== (incoming.status === "succeeded")) return true;
  if (query.status !== null && current.status !== incoming.status) return true;
  if (query.kind !== null && current.kind !== incoming.kind) return true;
  if (query.stage !== null && current.stage !== incoming.stage) return true;
  if (query.waitingReason !== null && current.waiting_reason !== incoming.waiting_reason) return true;
  if (query.dependencyKey !== null && current.dependency_key !== incoming.dependency_key) return true;
  if (query.sort !== null && current[query.sort.field] !== incoming[query.sort.field]) return true;

  // The server searches task and item payload fields, some of which are not in
  // TaskResponse. Resync conservatively rather than guessing search membership.
  return query.query.trim().length > 0;
}

export function createQueueUpdateRegistry(options: QueueUpdateRegistryOptions) {
  let pending = new Map<string, TaskResponse>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;

  function invalidate(): void {
    generation += 1;
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    pending.clear();
    pending = new Map();
  }

  function flush(windowGeneration: number): void {
    timer = null;
    if (windowGeneration !== generation || pending.size === 0) return;
    if (options.isLoading()) {
      invalidate();
      options.onLoadingUpdate();
      return;
    }

    const updates = pending;
    pending = new Map();
    const pageIds = new Set(options.items.value.map((task) => task.task_id));
    if ([...updates.keys()].some((taskId) => !pageIds.has(taskId))) {
      options.onStalePage();
      return;
    }

    options.items.value = options.items.value.map((task) => {
      const update = updates.get(task.task_id);
      return update ? { ...task, ...update } : task;
    });
  }

  function enqueue(incoming: TaskResponse): boolean {
    const current = options.items.value.find((task) => task.task_id === incoming.task_id);
    if (!current) {
      invalidate();
      options.onStalePage();
      return false;
    }
    if (options.isLoading()) {
      invalidate();
      options.onLoadingUpdate();
      return false;
    }
    if (!pending.has(incoming.task_id) && pending.size >= MAX_BUFFERED_TASKS) {
      invalidate();
      options.onStalePage();
      return false;
    }

    pending.set(incoming.task_id, { ...(pending.get(incoming.task_id) ?? current), ...incoming });
    if (timer === null) {
      const windowGeneration = generation;
      timer = setTimeout(() => flush(windowGeneration), UPDATE_WINDOW_MS);
    }
    return true;
  }

  return {
    enqueue,
    invalidate,
    get pendingCount() {
      return pending.size;
    },
  };
}
