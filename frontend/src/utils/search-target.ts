import type { LocationQuery } from "vue-router";

import type { SearchHit } from "../services/api";
import type { SearchFilters } from "../types/ui";
import { filtersToQuery } from "./search";

export const SEARCH_RETURN_QUERY_KEY = "from";
export const SEARCH_RETURN_QUERY_VALUE = "search";

export function buildSearchTarget(hit: SearchHit) {
  if (hit.is_library_file && hit.library_file_id) {
    return {
      name: "group-overview",
      params: {
        groupPath: hit.group_path,
      },
      query: {
        file: hit.library_file_id,
        [SEARCH_RETURN_QUERY_KEY]: SEARCH_RETURN_QUERY_VALUE,
      },
    };
  }

  return {
    name: "document",
    params: {
      id: hit.document_id,
    },
    query: {
      [SEARCH_RETURN_QUERY_KEY]: SEARCH_RETURN_QUERY_VALUE,
    },
  };
}

export function isSearchReturn(query: LocationQuery): boolean {
  const raw = query[SEARCH_RETURN_QUERY_KEY];
  const value = Array.isArray(raw) ? raw[0] : raw;
  return value === SEARCH_RETURN_QUERY_VALUE;
}

export function buildSearchReturnLocation(filters: SearchFilters, page = 1) {
  return {
    name: "search",
    query: filtersToQuery(filters, page),
  };
}
