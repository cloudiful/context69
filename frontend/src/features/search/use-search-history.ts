import { useCallback, useState } from "react";

import type { SearchFilters } from "@/types/ui";
import {
  addSearchHistoryEntry,
  readSearchHistory,
  type SearchHistoryEntry,
} from "@/utils/search-history";

const MAX_IN_MEMORY_ENTRIES = 20;
const MAX_VISIBLE_ENTRIES = 8;

function normalizeEntry(filters: SearchFilters): SearchFilters | null {
  const query = filters.query.trim();
  if (!query || query === "[object Object]") {
    return null;
  }
  return {
    query,
    sourceKey: filters.sourceKey,
    publishedAfter: filters.publishedAfter,
    publishedBefore: filters.publishedBefore,
    limit: Math.min(Math.max(filters.limit, 1), 50),
    groupPath: filters.groupPath ?? "",
    sort: filters.sort ?? "relevance",
  };
}

function sameEntry(left: SearchFilters, right: SearchFilters): boolean {
  return left.query === right.query
    && left.sourceKey === right.sourceKey
    && left.publishedAfter === right.publishedAfter
    && left.publishedBefore === right.publishedBefore
    && left.limit === right.limit
    && (left.groupPath ?? "") === (right.groupPath ?? "")
    && (left.sort ?? "relevance") === (right.sort ?? "relevance");
}

function mergeInMemory(
  current: SearchHistoryEntry[],
  filters: SearchFilters,
): SearchHistoryEntry[] {
  const normalized = normalizeEntry(filters);
  if (!normalized) {
    return current;
  }
  return [
    { ...normalized, savedAt: new Date().toISOString() },
    ...current.filter((entry) => !sameEntry(entry, normalized)),
  ].slice(0, MAX_IN_MEMORY_ENTRIES);
}

/**
 * Recent searches. Storage stays the source when it is readable; a denied or
 * unavailable store keeps the session's history in memory instead of failing
 * the page. Entries are always cursor-free so replaying one restarts page 1.
 */
export function useSearchHistory() {
  const [entries, setEntries] = useState<SearchHistoryEntry[]>(() => readSearchHistory());

  const add = useCallback((filters: SearchFilters) => {
    addSearchHistoryEntry(filters);
    const readBack = readSearchHistory();
    if (readBack.length > 0) {
      setEntries(readBack);
      return;
    }
    setEntries((current) => mergeInMemory(current, filters));
  }, []);

  return {
    entries: entries.slice(0, MAX_VISIBLE_ENTRIES),
    add,
  };
}
