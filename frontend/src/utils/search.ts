import type { LocationQuery, LocationQueryRaw } from "vue-router";

import type { SearchRequest } from "../services/api";
import type { SearchFilters } from "../types/ui";

function readQueryValue(value: LocationQuery[string]): string {
  if (Array.isArray(value)) {
    return value[0] || "";
  }

  return value || "";
}

export function createDefaultFilters(): SearchFilters {
  return {
    query: "",
    sourceKey: "",
    publishedAfter: "",
    publishedBefore: "",
    limit: 8,
  };
}

export function filtersFromQuery(query: LocationQuery): SearchFilters {
  const base = createDefaultFilters();
  const limit = Number.parseInt(readQueryValue(query.limit), 10);

  return {
    query: readQueryValue(query.q),
    sourceKey: readQueryValue(query.source),
    publishedAfter: readQueryValue(query.after),
    publishedBefore: readQueryValue(query.before),
    limit: Number.isFinite(limit) && limit > 0 ? Math.min(limit, 50) : base.limit,
  };
}

export function filtersToQuery(filters: SearchFilters): LocationQueryRaw {
  return {
    q: filters.query || undefined,
    source: filters.sourceKey || undefined,
    after: filters.publishedAfter || undefined,
    before: filters.publishedBefore || undefined,
    limit: filters.limit !== 8 ? String(filters.limit) : undefined,
  };
}

export function buildSearchPayload(filters: SearchFilters, page = 1): SearchRequest {
  return {
    query: filters.query.trim(),
    limit: Math.min(Math.max(filters.limit, 1), 50),
    page,
    source_key: filters.sourceKey || undefined,
    published_after: filters.publishedAfter || undefined,
    published_before: filters.publishedBefore || undefined,
  };
}
