export interface SearchFilters {
  query: string;
  sourceKey: string;
  publishedAfter: string;
  publishedBefore: string;
  limit: number;
}

export type AppTheme = "light" | "dark";
