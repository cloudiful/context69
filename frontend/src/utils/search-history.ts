import type { SearchFilters } from "../types/ui";

export const SEARCH_HISTORY_STORAGE_KEY = "context69.search-history";
const MAX_SEARCH_HISTORY_ITEMS = 20;

export interface SearchHistoryEntry extends SearchFilters {
  savedAt: string;
}

function getStorage(): Storage | null {
  if (typeof window === "undefined") {
    return null;
  }

  const storage = window.localStorage;
  return storage && typeof storage.getItem === "function" && typeof storage.setItem === "function"
    ? storage
    : null;
}

function normalizeFilters(filters: SearchFilters): SearchFilters {
  const query = filters.query.trim();
  return {
    query: query === "[object Object]" ? "" : query,
    sourceKey: filters.sourceKey,
    publishedAfter: filters.publishedAfter,
    publishedBefore: filters.publishedBefore,
    limit: Math.min(Math.max(filters.limit, 1), 50),
  };
}

function sameFilters(left: SearchFilters, right: SearchFilters) {
  return left.query === right.query
    && left.sourceKey === right.sourceKey
    && left.publishedAfter === right.publishedAfter
    && left.publishedBefore === right.publishedBefore
    && left.limit === right.limit;
}

export function readSearchHistory(storage: Storage | null | undefined = getStorage()): SearchHistoryEntry[] {
  const raw = storage?.getItem(SEARCH_HISTORY_STORAGE_KEY);
  if (!raw) {
    return [];
  }

  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }

    return parsed
      .map((entry) => {
        if (!entry || typeof entry !== "object") {
          return null;
        }

        const normalized = normalizeFilters({
          query: typeof entry.query === "string" ? entry.query : "",
          sourceKey: typeof entry.sourceKey === "string" ? entry.sourceKey : "",
          publishedAfter: typeof entry.publishedAfter === "string" ? entry.publishedAfter : "",
          publishedBefore: typeof entry.publishedBefore === "string" ? entry.publishedBefore : "",
          limit: typeof entry.limit === "number" ? entry.limit : 8,
        });

        if (!normalized.query) {
          return null;
        }

        return {
          ...normalized,
          savedAt: typeof entry.savedAt === "string" ? entry.savedAt : new Date(0).toISOString(),
        } satisfies SearchHistoryEntry;
      })
      .filter((entry): entry is SearchHistoryEntry => entry !== null)
      .slice(0, MAX_SEARCH_HISTORY_ITEMS);
  } catch {
    return [];
  }
}

export function writeSearchHistory(entries: SearchHistoryEntry[], storage: Storage | null | undefined = getStorage()) {
  storage?.setItem(SEARCH_HISTORY_STORAGE_KEY, JSON.stringify(entries.slice(0, MAX_SEARCH_HISTORY_ITEMS)));
}

export function addSearchHistoryEntry(
  filters: SearchFilters,
  storage: Storage | null | undefined = getStorage(),
): SearchHistoryEntry[] {
  const normalized = normalizeFilters(filters);
  if (!normalized.query) {
    return readSearchHistory(storage);
  }

  const nextEntry: SearchHistoryEntry = {
    ...normalized,
    savedAt: new Date().toISOString(),
  };

  const nextHistory = [
    nextEntry,
    ...readSearchHistory(storage).filter((entry) => !sameFilters(entry, normalized)),
  ].slice(0, MAX_SEARCH_HISTORY_ITEMS);

  writeSearchHistory(nextHistory, storage);
  return nextHistory;
}
