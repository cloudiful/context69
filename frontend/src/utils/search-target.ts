import type { SearchHit } from "../services/api";
import type { SearchFilters } from "../types/ui";
import { filtersToQuery } from "./search";

export const SEARCH_RETURN_QUERY_KEY = "from";
export const SEARCH_RETURN_QUERY_VALUE = "search";

/**
 * The in-app target of one hit. A library file opens its group screen with the
 * file selected; a document opens the document screen. Both carry the search
 * return marker so the destination can offer the way back.
 */
export function buildSearchTarget(hit: SearchHit): string {
  if (hit.is_library_file && hit.library_file_id) {
    const params = new URLSearchParams();
    params.set("file", hit.library_file_id);
    params.set(SEARCH_RETURN_QUERY_KEY, SEARCH_RETURN_QUERY_VALUE);
    return `/groups/${encodeURIComponent(hit.group_path)}?${params.toString()}`;
  }

  const params = new URLSearchParams();
  params.set(SEARCH_RETURN_QUERY_KEY, SEARCH_RETURN_QUERY_VALUE);
  return `/documents/${hit.document_id}?${params.toString()}`;
}

export function isSearchReturn(params: URLSearchParams): boolean {
  return params.get(SEARCH_RETURN_QUERY_KEY) === SEARCH_RETURN_QUERY_VALUE;
}

export function buildSearchReturnLocation(filters: SearchFilters, page = 1): string {
  const params = filtersToQuery(filters, page);
  const search = params.toString();
  return search ? `/search?${search}` : "/search";
}
