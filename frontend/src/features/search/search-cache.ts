import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useSyncExternalStore } from "react";

import type { SearchPage } from "./use-search-stream";

export const SEARCH_RESULTS_QUERY_ROOT = "search-results";

export function searchResultsKey(identity: string) {
  return [SEARCH_RESULTS_QUERY_ROOT, identity] as const;
}

/**
 * Reads the single cached search page for one filter/navigation identity. The
 * cache is the only page store; changing identity reads a different key, so a
 * page from another filter set or ordering window is never shown.
 */
export function useCachedSearchPage(identity: string): SearchPage | null {
  const queryClient = useQueryClient();
  const subscribe = useCallback(
    (listener: () => void) => queryClient.getQueryCache().subscribe(listener),
    [queryClient],
  );

  return useSyncExternalStore(
    subscribe,
    () => queryClient.getQueryData<SearchPage>(searchResultsKey(identity)) ?? null,
    () => null,
  );
}
