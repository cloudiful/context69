import type { LocationQuery, LocationQueryRaw } from "vue-router";

import type { GroupResponse, SearchRequest, SearchSort } from "../services/api";
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
    groupPath: "",
    sort: "relevance",
  };
}

function readSort(value: LocationQuery[string]): SearchSort | undefined {
  const raw = readQueryValue(value).toLowerCase();
  if (raw === "date" || raw === "relevance") {
    return raw;
  }
  return undefined;
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
    groupPath: readQueryValue(query.group_path),
    sort: readSort(query.sort) ?? base.sort,
  };
}

export function pageFromQuery(query: LocationQuery): number {
  const raw = readQueryValue(query.page);
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

/** Opaque pagination cursor carried in the `cursor` URL parameter. */
export function cursorFromQuery(query: LocationQuery): string | null {
  const raw = readQueryValue(query.cursor);
  return raw || null;
}

export interface SearchNavigationState {
  page?: number;
  cursor?: string | null;
}

/** Page number used when no cursor state is given. */
function navigationOf(value: number | SearchNavigationState | undefined): SearchNavigationState {
  if (typeof value === "number") {
    return { page: value };
  }
  return value ?? {};
}

export function filtersToQuery(
  filters: SearchFilters,
  navigation: number | SearchNavigationState = 1,
): LocationQueryRaw {
  const state = navigationOf(navigation);
  const page = state.page ?? 1;
  // A cursor pins the exact window in one ordering epoch; the legacy page
  // parameter is only serialized when no cursor is present.
  const hasCursor = !!state.cursor;
  // Date mode is forward-only keyset pagination: `page > 1` has no
  // meaning and would be rejected by the server with a 400. The client
  // must always navigate via `cursor` once a page 1 cursor is issued.
  const isDateMode = filters.sort === "date";
  return {
    q: filters.query || undefined,
    source: filters.sourceKey || undefined,
    after: filters.publishedAfter || undefined,
    before: filters.publishedBefore || undefined,
    limit: filters.limit !== 8 ? String(filters.limit) : undefined,
    page: hasCursor || page === 1 || isDateMode ? undefined : String(page),
    cursor: hasCursor ? state.cursor : undefined,
    group_path: filters.groupPath || undefined,
    sort: filters.sort && filters.sort !== "relevance" ? filters.sort : undefined,
  };
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

export const SEARCH_SESSION_STORAGE_KEY = "context69.search-session";

export interface SearchSessionState {
  filters: SearchFilters;
  page: number;
  cursor?: string | null;
}

export function saveSearchSession(
  filters: SearchFilters,
  page: number,
  cursor?: string | null,
): void {
  if (typeof window === "undefined" || !window.sessionStorage) return;
  try {
    const payload: SearchSessionState = {
      filters: normalizeSearchFilters(filters),
      page: Math.max(1, page),
      cursor: cursor || undefined,
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
    const cursor = typeof parsed.cursor === "string" ? parsed.cursor : null;
    if (!filters || typeof filters.query !== "string" || !filters.query.trim()) return null;
    return {
      filters: normalizeSearchFilters({
        query: typeof filters.query === "string" ? filters.query : "",
        sourceKey: typeof filters.sourceKey === "string" ? filters.sourceKey : "",
        publishedAfter: typeof filters.publishedAfter === "string" ? filters.publishedAfter : "",
        publishedBefore: typeof filters.publishedBefore === "string" ? filters.publishedBefore : "",
        limit: typeof filters.limit === "number" ? filters.limit : 8,
        groupPath: typeof filters.groupPath === "string" ? filters.groupPath : "",
        sort: filters.sort === "date" || filters.sort === "relevance" ? filters.sort : "relevance",
      }),
      page: Number.isFinite(page) && page > 0 ? page : 1,
      cursor: cursor || undefined,
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

export function buildSearchPayload(
  filters: SearchFilters,
  navigation: number | SearchNavigationState = 1,
): SearchRequest {
  const state = navigationOf(navigation);
  const page = state.page ?? 1;
  const hasCursor = !!state.cursor;
  const sort: SearchSort | undefined =
    filters.sort === "date"
      ? "date"
      : filters.sort === "relevance"
        ? "relevance"
        : undefined;
  return {
    query: filters.query.trim(),
    limit: Math.min(Math.max(filters.limit, 1), 50),
    page: hasCursor ? 1 : page,
    cursor: hasCursor ? state.cursor : undefined,
    source_key: filters.sourceKey || undefined,
    published_after: filters.publishedAfter || undefined,
    published_before: filters.publishedBefore || undefined,
    group_path: filters.groupPath || undefined,
    sort,
  };
}

export function escapeSearchHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

// Leading section labels tolerated after an embedded document title inside
// chunk_text (Chinese ingestion prefixes chunks with 标题：<title> followed by
// 摘要/摘要片段/正文 sections; English chunks use a similar `Title:` line).
const TITLE_PREFIX_PATTERN = "(?:标题|title)";
const SECTION_LABEL_PATTERN = "(?:摘要片段|摘要|正文|来源|日期|summary|body|source|date)";

function escapeSearchRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Removes a leading "标题：<title>" / "Title: <title>" prefix from a chunk when
 * the embedded title matches `title` (whitespace runs and ASCII case are
 * normalized). The remainder starts at the first following content section, so
 * labels such as 摘要：/正文：/Summary: are preserved. When no title prefix
 * matches, the text is returned unchanged.
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

export function highlightQueryText(text: string, query: string): string {
  const term = escapeSearchHtml(query.trim());
  const escaped = escapeSearchHtml(text);
  if (!term) {
    return escaped;
  }
  return escaped.split(term).join(`<mark>${term}</mark>`);
}

export function buildSnippet(text: string, query: string, radius = 90): string {
  const needle = query.trim();
  if (needle) {
    const index = text.toLowerCase().indexOf(needle.toLowerCase());
    if (index >= 0) {
      const start = Math.max(0, index - radius);
      const end = Math.min(text.length, index + needle.length + radius);
      return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
    }
  }
  return text.length > radius * 2 ? `${text.slice(0, radius * 2)}…` : text;
}

export function matchReasonTokens(matchReason: string | null | undefined): string[] {
  return (matchReason ?? "").split("+").filter((token) => token.length > 0);
}

export const GROUP_FOLDER_ALL_VALUE = "__all__";

export interface GroupFolderOption {
  label: string;
  value: string;
}

/**
 * Flattens the accessible group list into a single-select "tree" by sorting on
 * group_path and indenting each entry by its depth. A group path is its own
 * value; the empty string is not represented here so the caller can prepend an
 * "all groups" option.
 */
export function groupFolderOptions(groups: GroupResponse[]): GroupFolderOption[] {
  const sorted = [...groups]
    .filter((group) => !!group.group_path)
    .sort((left, right) => (left.group_path ?? "").localeCompare(right.group_path ?? ""));

  return sorted.map((group) => {
    const segments = (group.group_path ?? "").split("/").filter(Boolean);
    const depth = Math.max(0, segments.length - 1);
    return {
      value: group.group_path ?? "",
      label: `${"  ".repeat(depth)}${segments.join(" / ")}`,
    };
  });
}
