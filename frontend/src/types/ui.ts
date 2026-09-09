export type SearchSortMode = "relevance" | "date";

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
  sort?: SearchSortMode;
}

export type AppTheme = "light" | "dark";
