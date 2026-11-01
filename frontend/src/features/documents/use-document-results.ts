import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { apiClient } from "@/services/api";
import type { DocumentResultResponse } from "@/services/api";

export type DocumentResultsView = "current" | "history";

export interface HistoryResult {
  result: DocumentResultResponse;
  isCurrent: boolean;
}

export interface DocumentResultKindGroup {
  resultKind: string;
  items: DocumentResultResponse[];
}

export interface DocumentResultProducerGroup {
  producer: string;
  kinds: DocumentResultKindGroup[];
}

const RESULTS_PAGE_LIMIT = 50;

export function groupCurrentResults(items: DocumentResultResponse[]): DocumentResultProducerGroup[] {
  const producers = new Map<string, Map<string, DocumentResultResponse[]>>();
  for (const item of items) {
    const kinds = producers.get(item.producer) ?? new Map<string, DocumentResultResponse[]>();
    kinds.set(item.result_kind, [...(kinds.get(item.result_kind) ?? []), item]);
    producers.set(item.producer, kinds);
  }
  return [...producers.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([producer, kinds]) => ({
      producer,
      kinds: [...kinds.entries()]
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([resultKind, kindItems]) => ({ resultKind, items: kindItems })),
    }));
}

/**
 * One document's caller-produced results. The default view asks only for the
 * newest row per producer and kind in the document's current source version;
 * both views read one bounded page at a time, so a document with more producer
 * or kind groups than one page keeps each group reachable.
 */
export function useDocumentResults(documentId: number | null, groupPath: string | null) {
  const [view, setView] = useState<DocumentResultsView>("current");
  // Bumped when a current page reports another source version. A new key
  // restarts bounded pagination at the first page, so rows of the previous
  // version can never stay displayed as Current and an obsolete in-flight page
  // can never merge into the new version.
  const [currentEpoch, setCurrentEpoch] = useState(0);
  const enabled = documentId !== null && groupPath !== null;

  const currentQuery = useInfiniteQuery({
    queryKey: ["document-results", documentId, groupPath, "current", currentEpoch],
    queryFn: ({ pageParam, signal }) =>
      apiClient.listDocumentResults(
        groupPath as string,
        documentId as number,
        {
          current_only: true,
          latest_only: true,
          limit: RESULTS_PAGE_LIMIT,
          after_id: pageParam ?? undefined,
        },
        { signal },
      ),
    initialPageParam: null as number | null,
    getNextPageParam: (lastPage) => lastPage.next_after_id ?? null,
    enabled: enabled && view === "current",
    retry: false,
  });

  const historyQuery = useInfiniteQuery({
    queryKey: ["document-results", documentId, groupPath, "history"],
    queryFn: ({ pageParam, signal }) =>
      apiClient.listDocumentResults(
        groupPath as string,
        documentId as number,
        {
          limit: RESULTS_PAGE_LIMIT,
          after_id: pageParam ?? undefined,
        },
        { signal },
      ),
    initialPageParam: null as number | null,
    getNextPageParam: (lastPage) => lastPage.next_after_id ?? null,
    enabled: enabled && view === "history",
    retry: false,
  });

  const currentPages = currentQuery.data?.pages ?? [];
  const activeSourceRecordHash = currentPages[0]?.source_record_hash ?? null;
  useEffect(() => {
    const moved = currentPages.some(
      (page) => page.source_record_hash !== activeSourceRecordHash,
    );
    if (moved) {
      setCurrentEpoch((value) => value + 1);
    }
  }, [currentPages, activeSourceRecordHash]);

  const currentItems = new Map<number, DocumentResultResponse>();
  for (const page of currentPages) {
    for (const item of page.items) {
      currentItems.set(item.id, item);
    }
  }
  const currentGroups = groupCurrentResults([...currentItems.values()]);
  const history: HistoryResult[] = (historyQuery.data?.pages ?? []).flatMap((page) =>
    page.items.map((result) => ({
      result,
      isCurrent: result.source_record_hash === page.source_record_hash,
    })),
  );

  return {
    view,
    setView,
    currentSourceRecordHash: activeSourceRecordHash,
    current: {
      groups: currentGroups,
      isLoading: currentQuery.isLoading,
      isFetching: currentQuery.isFetching,
      isFetchingNextPage: currentQuery.isFetchingNextPage,
      hasNextPage: currentQuery.hasNextPage,
      error: currentQuery.error,
      reload: () => {
        void currentQuery.refetch();
      },
      loadMore: () => {
        void currentQuery.fetchNextPage();
      },
    },
    history: {
      items: history,
      isLoading: historyQuery.isLoading,
      isFetching: historyQuery.isFetching,
      isFetchingNextPage: historyQuery.isFetchingNextPage,
      hasNextPage: historyQuery.hasNextPage,
      error: historyQuery.error,
      reload: () => {
        void historyQuery.refetch();
      },
      loadMore: () => {
        void historyQuery.fetchNextPage();
      },
    },
  };
}
