import type { GroupResponse, SearchRequest, SearchSort } from "../services/api";
import type { SearchFilters } from "../types/ui";
import {
  escapeSearchRegExp,
  findSearchHighlightRanges,
  highlightQueryText,
  searchHighlightTerms,
} from "./search-highlight";

/** Re-exported so highlighting callers keep a single search utility entry point. */
export { highlightQueryText };

export const DEFAULT_SEARCH_LIMIT = 8;
export const SEARCH_SESSION_STORAGE_KEY = "context69.search-session";
export const GROUP_FOLDER_ALL_VALUE = "__all__";

export interface SearchNavigationState {
  page?: number;
  cursor?: string | null;
}

export function createDefaultFilters(): SearchFilters {
  return {
    query: "",
    sourceKey: "",
    publishedAfter: "",
    publishedBefore: "",
    limit: DEFAULT_SEARCH_LIMIT,
    groupPath: "",
    sort: "relevance",
  };
}

function readParam(params: URLSearchParams, key: string): string {
  return params.get(key) ?? "";
}

function readSort(value: string): SearchSort {
  const raw = value.toLowerCase();
  return raw === "date" || raw === "relevance" ? raw : "relevance";
}

/** The URL contract of the search page. */
export function filtersFromQuery(params: URLSearchParams): SearchFilters {
  const limit = Number.parseInt(readParam(params, "limit"), 10);

  return {
    query: readParam(params, "q"),
    sourceKey: readParam(params, "source"),
    publishedAfter: readParam(params, "after"),
    publishedBefore: readParam(params, "before"),
    limit: Number.isFinite(limit) && limit > 0 ? Math.min(limit, 50) : DEFAULT_SEARCH_LIMIT,
    groupPath: readParam(params, "group_path"),
    sort: readSort(readParam(params, "sort")),
  };
}

export function pageFromQuery(params: URLSearchParams): number {
  const parsed = Number.parseInt(readParam(params, "page"), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

/** Opaque pagination cursor carried in the `cursor` URL parameter. */
export function cursorFromQuery(params: URLSearchParams): string | null {
  const raw = readParam(params, "cursor");
  return raw || null;
}

export function navigationFromQuery(params: URLSearchParams): SearchNavigationState {
  const cursor = cursorFromQuery(params);
  return cursor ? { cursor } : { page: pageFromQuery(params) };
}

function navigationOf(value: number | SearchNavigationState | undefined): SearchNavigationState {
  if (typeof value === "number") {
    return { page: value };
  }
  return value ?? {};
}

export function filtersToQuery(
  filters: SearchFilters,
  navigation: number | SearchNavigationState = 1,
): URLSearchParams {
  const state = navigationOf(navigation);
  const page = state.page ?? 1;
  const hasCursor = !!state.cursor;
  const isDateMode = filters.sort === "date";
  const params = new URLSearchParams();

  if (filters.query) params.set("q", filters.query);
  if (filters.sourceKey) params.set("source", filters.sourceKey);
  if (filters.publishedAfter) params.set("after", filters.publishedAfter);
  if (filters.publishedBefore) params.set("before", filters.publishedBefore);
  if (filters.limit !== DEFAULT_SEARCH_LIMIT) params.set("limit", String(filters.limit));
  if (!hasCursor && page !== 1 && !isDateMode) params.set("page", String(page));
  if (hasCursor) params.set("cursor", state.cursor as string);
  if (filters.groupPath) params.set("group_path", filters.groupPath);
  if (filters.sort === "date") params.set("sort", "date");

  return params;
}

export function sameSearchFilters(a: SearchFilters, b: SearchFilters): boolean {
  return a.query === b.query
    && a.sourceKey === b.sourceKey
    && a.publishedAfter === b.publishedAfter
    && a.publishedBefore === b.publishedBefore
    && a.limit === b.limit
    && (a.groupPath ?? "") === (b.groupPath ?? "")
    && (a.sort ?? "relevance") === (b.sort ?? "relevance");
}

export function normalizeSearchFilters(filters: SearchFilters): SearchFilters {
  return {
    query: filters.query.trim(),
    sourceKey: filters.sourceKey,
    publishedAfter: filters.publishedAfter,
    publishedBefore: filters.publishedBefore,
    limit: Math.min(Math.max(filters.limit, 1), 50),
    groupPath: filters.groupPath ?? "",
    sort: filters.sort ?? "relevance",
  };
}

/** Stable cache identity for one filter set and one navigation window. */
export function searchIdentity(
  filters: SearchFilters,
  navigation: SearchNavigationState,
): string {
  return JSON.stringify({
    filters: normalizeSearchFilters(filters),
    cursor: navigation.cursor ?? null,
    page: navigation.cursor ? 1 : navigation.page ?? 1,
  });
}

export function buildSearchPayload(
  filters: SearchFilters,
  navigation: number | SearchNavigationState = 1,
): SearchRequest {
  const state = navigationOf(navigation);
  const page = state.page ?? 1;
  const hasCursor = !!state.cursor;

  return {
    query: filters.query.trim(),
    limit: Math.min(Math.max(filters.limit, 1), 50),
    page: hasCursor ? 1 : page,
    cursor: hasCursor ? state.cursor : undefined,
    source_key: filters.sourceKey || undefined,
    published_after: filters.publishedAfter || undefined,
    published_before: filters.publishedBefore || undefined,
    group_path: filters.groupPath || undefined,
    sort: filters.sort,
  };
}

export interface SearchSessionState {
  filters: SearchFilters;
  page: number;
  cursor?: string | null;
}

/**
 * Lightweight return context for the document or group screen: filters and the
 * ordering position only, never result chunks or query payload contents.
 */
export function saveSearchSession(
  filters: SearchFilters,
  page: number,
  cursor?: string | null,
): void {
  if (typeof window === "undefined") {
    return;
  }

  try {
    const payload: SearchSessionState = {
      filters: normalizeSearchFilters(filters),
      page: Math.max(1, page),
      cursor: cursor || undefined,
    };
    if (!payload.filters.query) {
      return;
    }
    window.sessionStorage.setItem(SEARCH_SESSION_STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // Session storage is optional return context; a denied store is harmless.
  }
}

export function loadSearchSession(): SearchSessionState | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    const raw = window.sessionStorage.getItem(SEARCH_SESSION_STORAGE_KEY);
    if (!raw) {
      return null;
    }
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") {
      return null;
    }
    const candidate = parsed as { filters?: unknown; page?: unknown; cursor?: unknown };
    const filters = candidate.filters;
    if (!filters || typeof filters !== "object") {
      return null;
    }
    const record = filters as Record<string, unknown>;
    if (typeof record.query !== "string" || !record.query.trim()) {
      return null;
    }
    return {
      filters: normalizeSearchFilters({
        query: record.query,
        sourceKey: typeof record.sourceKey === "string" ? record.sourceKey : "",
        publishedAfter: typeof record.publishedAfter === "string" ? record.publishedAfter : "",
        publishedBefore: typeof record.publishedBefore === "string" ? record.publishedBefore : "",
        limit: typeof record.limit === "number" ? record.limit : DEFAULT_SEARCH_LIMIT,
        groupPath: typeof record.groupPath === "string" ? record.groupPath : "",
        sort: record.sort === "date" ? "date" : "relevance",
      }),
      page: typeof candidate.page === "number" && candidate.page > 0 ? candidate.page : 1,
      cursor: typeof candidate.cursor === "string" ? candidate.cursor : undefined,
    };
  } catch {
    return null;
  }
}

// Leading section labels tolerated after an embedded document title inside
// chunk_text (Chinese ingestion prefixes chunks with 标题：<title> followed by
// 摘要/摘要片段/正文 sections; English chunks use a similar `Title:` line).
const TITLE_PREFIX_PATTERN = "(?:标题|title)";
const SECTION_LABEL_PATTERN = "(?:摘要片段|摘要|正文|来源|日期|summary|body|source|date)";

/**
 * Removes a leading "标题：<title>" / "Title: <title>" prefix from a chunk when
 * the embedded title matches `title` (whitespace runs and ASCII case are
 * normalized). Labels such as 摘要：/正文：/Summary: are preserved.
 */
export function stripTitlePrefix(chunkText: string, title: string): string {
  if (!chunkText) {
    return chunkText;
  }
  const normalizedTitle = title.trim().replace(/\s+/g, " ");
  if (!normalizedTitle) {
    return chunkText;
  }
  const titlePattern = normalizedTitle.split(" ").map(escapeSearchRegExp).join("\\s+");
  const match = new RegExp(
    `^\\s*${TITLE_PREFIX_PATTERN}\\s*[:：]\\s*(${titlePattern})(?=\\s|$|${SECTION_LABEL_PATTERN})`,
    "i",
  ).exec(chunkText);
  if (!match) {
    return chunkText;
  }
  return chunkText.slice(match.index + match[0].length).replace(/^\s+/, "");
}

export function buildSnippet(text: string, query: string, radius = 90): string {
  const needle = query.trim();
  if (needle) {
    // Prefer the whole phrase when it occurs literally, and otherwise fall back
    // to the earliest matched term, so a multi-word query still windows the
    // snippet around what the user actually typed.
    const phrase = findSearchHighlightRanges(text, [needle])[0];
    const first = phrase ?? findSearchHighlightRanges(text, searchHighlightTerms(query))[0];
    if (first) {
      const start = Math.max(0, first.start - radius);
      const end = Math.min(text.length, first.end + radius);
      return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
    }
  }
  return text.length > radius * 2 ? `${text.slice(0, radius * 2)}…` : text;
}

export function matchReasonTokens(matchReason: string | null | undefined): string[] {
  return (matchReason ?? "").split("+").filter((token) => token.length > 0);
}

export interface GroupFolderOption {
  label: string;
  value: string;
  depth: number;
}

/**
 * Flattens the accessible group list into a single-select "tree" by sorting on
 * group_path and exposing each entry's depth for indentation.
 */
export function groupFolderOptions(groups: GroupResponse[]): GroupFolderOption[] {
  return [...groups]
    .filter((group) => !!group.group_path)
    .sort((left, right) => (left.group_path ?? "").localeCompare(right.group_path ?? ""))
    .map((group) => {
      const groupPath = group.group_path ?? "";
      const segments = groupPath.split("/").filter(Boolean);
      return {
        value: groupPath,
        label: segments.join(" / "),
        depth: Math.max(0, segments.length - 1),
      };
    });
}
