import type { SearchSort } from "../services/api/api-types";

/**
 * Deprecated alias kept so out-of-scope callers (e.g. SearchForm.vue) keep
 * compiling during the Task 6 migration window. New code must import
 * `SearchSort` from the canonical generated OpenAPI types directly.
 */
export type SearchSortMode = SearchSort;

export interface SearchFilters {
  query: string;
  sourceKey: string;
  publishedAfter: string;
  publishedBefore: string;
  limit: number;
  groupPath?: string | null;
  /**
   * Ordering applied to the search. `relevance` (default) is the existing
   * vector/hybrid + rerank pipeline. `date` is a latest-first walk over
   * non-overlapping `published_ts` windows without rerank.
   */
  sort?: SearchSort;
}

export type AppTheme = "light" | "dark";
