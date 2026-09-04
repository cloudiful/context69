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

export function pageFromQuery(query: LocationQuery): number {
  const raw = readQueryValue(query.page);
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

export function filtersToQuery(filters: SearchFilters, page = 1): LocationQueryRaw {
  return {
    q: filters.query || undefined,
    source: filters.sourceKey || undefined,
    after: filters.publishedAfter || undefined,
    before: filters.publishedBefore || undefined,
    limit: filters.limit !== 8 ? String(filters.limit) : undefined,
    page: page !== 1 ? String(page) : undefined,
  };
}

export function sameSearchFilters(a: SearchFilters, b: SearchFilters): boolean {
  return a.query === b.query
    && a.sourceKey === b.sourceKey
    && a.publishedAfter === b.publishedAfter
    && a.publishedBefore === b.publishedBefore
    && a.limit === b.limit;
}

export function normalizeSearchFilters(filters: SearchFilters): SearchFilters {
  return {
    query: filters.query.trim(),
    sourceKey: filters.sourceKey,
    publishedAfter: filters.publishedAfter,
    publishedBefore: filters.publishedBefore,
    limit: Math.min(Math.max(filters.limit, 1), 50),
  };
}

export const SEARCH_SESSION_STORAGE_KEY = "context69.search-session";

export interface SearchSessionState {
  filters: SearchFilters;
  page: number;
}

export function saveSearchSession(filters: SearchFilters, page: number): void {
  if (typeof window === "undefined" || !window.sessionStorage) return;
  try {
    const payload: SearchSessionState = {
      filters: normalizeSearchFilters(filters),
      page: Math.max(1, page),
    };
    if (!payload.filters.query) return;
    window.sessionStorage.setItem(SEARCH_SESSION_STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // ignore storage errors
  }
}

export function loadSearchSession(): SearchSessionState | null {
  if (typeof window === "undefined" || !window.sessionStorage) return null;
  try {
    const raw = window.sessionStorage.getItem(SEARCH_SESSION_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const filters = parsed.filters;
    const page = typeof parsed.page === "number" ? parsed.page : 1;
    if (!filters || typeof filters.query !== "string" || !filters.query.trim()) return null;
    return {
      filters: normalizeSearchFilters({
        query: typeof filters.query === "string" ? filters.query : "",
        sourceKey: typeof filters.sourceKey === "string" ? filters.sourceKey : "",
        publishedAfter: typeof filters.publishedAfter === "string" ? filters.publishedAfter : "",
        publishedBefore: typeof filters.publishedBefore === "string" ? filters.publishedBefore : "",
        limit: typeof filters.limit === "number" ? filters.limit : 8,
      }),
      page: Number.isFinite(page) && page > 0 ? page : 1,
    };
  } catch {
    return null;
  }
}

export function clearSearchSession(): void {
  if (typeof window === "undefined" || !window.sessionStorage) return;
  try {
    window.sessionStorage.removeItem(SEARCH_SESSION_STORAGE_KEY);
  } catch {
    // ignore
  }
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
