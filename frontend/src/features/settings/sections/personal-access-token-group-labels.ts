import { useSyncExternalStore } from "react";

import { apiClient, type GroupResponse } from "@/services/api";

const GROUP_PAGE_SIZE = 100;

export interface TokenGroupLabel {
  name: string;
  path: string;
}

interface LabelStore {
  cache: Map<number, TokenGroupLabel>;
  attempted: Set<number>;
  inFlight: Set<number>;
  version: number;
  listeners: Set<() => void>;
}

const stores = new Map<string, LabelStore>();

function storeFor(userKey: string): LabelStore {
  const existing = stores.get(userKey);
  if (existing) {
    return existing;
  }
  const store: LabelStore = {
    cache: new Map(),
    attempted: new Set(),
    inFlight: new Set(),
    version: 0,
    listeners: new Set(),
  };
  stores.set(userKey, store);
  return store;
}

function pathOf(group: GroupResponse): string {
  return group.group_path ?? group.group_key;
}

function emit(store: LabelStore) {
  store.version += 1;
  for (const listener of store.listeners) {
    listener();
  }
}

export function rememberGroups(userKey: string, groups: GroupResponse[]) {
  const store = storeFor(userKey);
  let changed = false;
  for (const group of groups) {
    const next = { name: group.name, path: pathOf(group) };
    const previous = store.cache.get(group.group_id);
    if (!previous || previous.name !== next.name || previous.path !== next.path) {
      store.cache.set(group.group_id, next);
      changed = true;
    }
  }
  if (changed) {
    emit(store);
  }
}

/** Per-user cache of group names observed through paged browsing or bounded search. */
export function useGroupLabelCache(userKey: string): ReadonlyMap<number, TokenGroupLabel> {
  const store = storeFor(userKey);
  useSyncExternalStore(
    (listener) => {
      store.listeners.add(listener);
      return () => {
        store.listeners.delete(listener);
      };
    },
    () => store.version,
    () => store.version,
  );
  return store.cache;
}

/**
 * Best-effort resolution of referenced group ids through the real paged group
 * tree. It stops as soon as every requested id is found, records what it
 * learned for this user, and leaves unreachable ids to the stable `Group #id`
 * fallback.
 */
export async function resolveGroupLabels(
  userKey: string,
  ids: number[],
  cancelled: () => boolean,
): Promise<void> {
  const store = storeFor(userKey);
  const pending = ids.filter(
    (id) => !store.cache.has(id) && !store.attempted.has(id) && !store.inFlight.has(id),
  );
  if (pending.length === 0) {
    return;
  }
  for (const id of pending) {
    store.inFlight.add(id);
  }

  const remaining = new Set(pending);

  function handle(groups: GroupResponse[]) {
    rememberGroups(userKey, groups);
    for (const group of groups) {
      remaining.delete(group.group_id);
    }
  }

  let failed = false;
  try {
    let page = 1;
    let pages = 1;
    do {
      const response = await apiClient.listPersonalAccessTokenGroupCatalog({
        page,
        pageSize: GROUP_PAGE_SIZE,
        query: "",
      });
      handle(response.items);
      pages = response.pagination.total_pages;
      page += 1;
    } while (remaining.size > 0 && page <= pages && !cancelled());
  } catch {
    // Label resolution is presentational; authorization stays backend-owned.
    failed = true;
  } finally {
    for (const id of pending) {
      store.inFlight.delete(id);
      if (!failed && !cancelled()) {
        store.attempted.add(id);
      }
    }
  }
}
