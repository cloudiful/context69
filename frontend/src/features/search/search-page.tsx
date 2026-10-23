import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearchParams } from "react-router";
import { toast } from "sonner";

import { resolveSearchErrorMessage } from "@/utils/search-errors";
import { replaySearchEntry, type SearchHistoryEntry } from "@/utils/search-history";
import { apiClient } from "@/services/api";
import type { SearchHit } from "@/services/api";

import { SEARCH_RESULTS_QUERY_ROOT, searchResultsKey, useCachedSearchPage } from "./search-cache";
import { SearchForm } from "./search-form";
import { SearchPreview } from "./search-preview";
import {
  buildSearchPayload,
  createDefaultFilters,
  cursorFromQuery,
  filtersFromQuery,
  filtersToQuery,
  normalizeSearchFilters,
  pageFromQuery,
  saveSearchSession,
  searchIdentity,
  type SearchNavigationState,
} from "@/utils/search";
import { SearchResults } from "./search-results";
import { buildSearchTarget } from "@/utils/search-target";
import { useSearchHistory } from "./use-search-history";
import { isCursorOrderingError, useSearchStream, type SearchPage } from "./use-search-stream";

interface UrlState {
  filters: ReturnType<typeof filtersFromQuery>;
  navigation: SearchNavigationState;
}

interface ExecuteOptions {
  filters: ReturnType<typeof createDefaultFilters>;
  navigation?: SearchNavigationState;
  syncRoute?: boolean;
  recordHistory?: boolean;
}

/**
 * The search screen. The URL is the only filter and page state, the React Query
 * cache holds exactly the current page for its filter/navigation identity, and
 * the streaming controller owns request generation and cancellation.
 */
export function SearchPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const searchParamsString = searchParams.toString();

  const urlState = useMemo<UrlState>(() => {
    const params = new URLSearchParams(searchParamsString);
    return {
      filters: filtersFromQuery(params),
      navigation: cursorFromQuery(params)
        ? { cursor: cursorFromQuery(params) }
        : { page: pageFromQuery(params) },
    };
  }, [searchParamsString]);
  const urlFilters = urlState.filters;
  const urlNavigation = urlState.navigation;
  const identity = useMemo(
    () => searchIdentity(urlFilters, urlNavigation),
    [urlFilters, urlNavigation],
  );

  const [draft, setDraft] = useState(urlFilters);
  const [selectedHit, setSelectedHit] = useState<SearchHit | null>(null);
  const [previewHit, setPreviewHit] = useState<SearchHit | null>(null);
  const searchHistory = useSearchHistory();

  const lastAppliedRef = useRef<string | null>(null);
  const activeIdentityRef = useRef<string | null>(null);
  const appliedNavigationRef = useRef<SearchNavigationState>({ page: 1 });

  const page = useCachedSearchPage(identity);

  const handlePage = useCallback((next: SearchPage) => {
    const runIdentity = activeIdentityRef.current;
    if (!runIdentity) {
      return;
    }
    queryClient.setQueryData(searchResultsKey(runIdentity), next);
    setSelectedHit(null);
    setPreviewHit(null);
    queryClient.removeQueries({
      queryKey: [SEARCH_RESULTS_QUERY_ROOT],
      predicate: (query) => query.queryKey[1] !== runIdentity,
    });
  }, [queryClient]);

  const { loading, refining, fallbackNotice, run: runSearch, abort: abortSearch } = useSearchStream(handlePage);
  const addHistory = searchHistory.add;

  const showError = useCallback((description: string) => {
    toast.error(t("common.error"), { description, duration: 5000 });
  }, [t]);

  const execute = useCallback(async ({
    filters,
    navigation = { page: 1 },
    syncRoute = true,
    recordHistory = false,
  }: ExecuteOptions) => {
    const normalized = normalizeSearchFilters(filters);
    const payload = buildSearchPayload(normalized, navigation);

    if (!payload.query) {
      if (syncRoute) {
        showError(t("search.emptyQuery"));
      }
      return;
    }

    const runIdentity = searchIdentity(normalized, navigation);
    lastAppliedRef.current = runIdentity;
    activeIdentityRef.current = runIdentity;
    appliedNavigationRef.current = navigation;
    if (syncRoute) {
      setSearchParams(filtersToQuery(normalized, navigation), { replace: true });
    }
    saveSearchSession(normalized, payload.page ?? 1, navigation.cursor ?? null);
    setSelectedHit(null);
    setPreviewHit(null);
    if (recordHistory) {
      addHistory(normalized);
    }

    try {
      await runSearch(payload);
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        return;
      }
      if (isCursorOrderingError(error)) {
        // The cursor belongs to another ordering epoch (for example rerank
        // availability changed). Restart from the first page instead of
        // silently showing a differently ordered window.
        showError(t("search.streaming.cursorError"));
        await execute({ filters: normalized, navigation: { page: 1 }, syncRoute: true });
        return;
      }
      showError(resolveSearchErrorMessage(error, t));
    }
  }, [addHistory, runSearch, setSearchParams, showError, t]);

  // Run the URL state on mount and whenever it changes from outside the form
  // (deep link, back/forward, a paging link).
  useEffect(() => {
    if (!urlFilters.query) {
      setSelectedHit(null);
      setPreviewHit(null);
      return;
    }
    if (lastAppliedRef.current === identity) {
      return;
    }
    lastAppliedRef.current = identity;
    void execute({ filters: urlFilters, navigation: urlNavigation, syncRoute: false });
  }, [execute, identity, urlFilters, urlNavigation]);

  // An external URL change is the applied state; mirror it into the form unless
  // the user is still editing a draft for the same identity.
  useEffect(() => {
    setDraft(urlFilters);
  }, [identity]);

  const sourcesQuery = useQuery({
    queryKey: ["search-sources"],
    queryFn: () => apiClient.listSources({ page: 1, pageSize: 100, query: "" }),
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const groupsQuery = useQuery({
    queryKey: ["search-groups"],
    queryFn: () => apiClient.listGroups({ page: 1, page_size: 100, query: "" }),
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });

  useEffect(() => {
    if (sourcesQuery.error) {
      showError(t("search.sourceLoadFailed"));
    }
  }, [sourcesQuery.error, showError, t]);

  useEffect(() => {
    if (groupsQuery.error) {
      showError(t("search.form.groupLoadFailed"));
    }
  }, [groupsQuery.error, showError, t]);

  const pagination = page?.pagination ?? null;

  const rerun = (filters: ReturnType<typeof createDefaultFilters>, navigation: SearchNavigationState) => {
    void execute({ filters, navigation, syncRoute: true });
  };

  const submit = () => {
    void execute({ filters: draft, navigation: { page: 1 }, syncRoute: true, recordHistory: true });
  };

  const changeGroup = (groupPath: string) => {
    const next = { ...draft, groupPath };
    setDraft(next);
    void execute({ filters: next, navigation: { page: 1 }, syncRoute: true });
  };

  const replayHistory = (entry: SearchHistoryEntry) => {
    const filters = replaySearchEntry(entry);
    setDraft(filters);
    void execute({ filters, navigation: { page: 1 }, syncRoute: true });
  };

  const goPrev = () => {
    if (!pagination || draft.sort === "date") {
      return;
    }
    if (pagination.prev_cursor) {
      rerun(draft, { cursor: pagination.prev_cursor });
    } else if (pagination.page > 1) {
      rerun(draft, { page: pagination.page - 1 });
    }
  };

  const goNext = () => {
    if (!pagination) {
      return;
    }
    if (pagination.next_cursor) {
      rerun(draft, { cursor: pagination.next_cursor });
    } else if (pagination.has_more === true) {
      rerun(draft, { page: pagination.page + 1 });
    }
  };

  const changePageSize = (size: number) => {
    const next = { ...draft, limit: Math.min(Math.max(size, 1), 50) };
    setDraft(next);
    void execute({ filters: next, navigation: { page: 1 }, syncRoute: true });
  };

  const openHit = (hit: SearchHit) => {
    saveSearchSession(
      draft,
      pagination?.page ?? 1,
      appliedNavigationRef.current.cursor ?? null,
    );
    setPreviewHit(null);
    navigate(buildSearchTarget(hit));
  };

  const selectHit = (hit: SearchHit) => {
    setSelectedHit(hit);
    setPreviewHit(hit);
  };

  const showPanel = loading || refining || page !== null;

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col gap-2 overflow-hidden">
      <h1 className="text-lg font-semibold">{t("search.consoleTitle")}</h1>
      <SearchForm
        filters={draft}
        sources={sourcesQuery.data?.items ?? []}
        groups={groupsQuery.data?.items ?? []}
        history={searchHistory.entries}
        busy={loading}
        onChange={setDraft}
        onSubmit={submit}
        onGroupChange={changeGroup}
        onHistorySelect={replayHistory}
        onReset={() => setDraft(createDefaultFilters())}
      />

      {showPanel ? (
        <SearchResults
          items={page?.items ?? []}
          pagination={pagination}
          query={urlFilters.query}
          loading={loading}
          refining={refining}
          fallbackNotice={fallbackNotice}
          searched={page !== null}
          sort={urlFilters.sort ?? "relevance"}
          selectedChunkId={selectedHit?.chunk_id ?? null}
          onSelect={selectHit}
          onOpen={openHit}
          onCancel={abortSearch}
          onPrev={goPrev}
          onNext={goNext}
          onPageSize={changePageSize}
        />
      ) : null}

      <SearchPreview
        hit={previewHit}
        query={urlFilters.query}
        open={previewHit !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPreviewHit(null);
          }
        }}
        onOpen={openHit}
      />
    </div>
  );
}
