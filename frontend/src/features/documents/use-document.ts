import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams, useSearchParams } from "react-router";

import { apiClient } from "@/services/api";
import type { DocumentResponse } from "@/services/api";
import { loadSearchSession } from "@/utils/search";
import { buildSearchReturnLocation, isSearchReturn } from "@/utils/search-target";

import { documentErrorKind, documentLibraryTarget, parseDocumentId, safeExternalUrl } from "./document-model";

/**
 * Owns one document read. The query key is the validated numeric id, so a route
 * change fences the previous response and aborts its request instead of leaving
 * the old document on screen.
 */
export function useDocument() {
  const params = useParams();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const searchString = searchParams.toString();

  const documentId = parseDocumentId(params.id);
  const fromSearch = useMemo(
    () => isSearchReturn(new URLSearchParams(searchString)),
    [searchString],
  );

  // Return context only: the stored session keeps the query the reader arrived
  // with. A denied store yields no highlight instead of blocking the read.
  const [highlightQuery] = useState(() => {
    if (!fromSearch) {
      return "";
    }
    return loadSearchSession()?.filters.query ?? "";
  });

  const abortRef = useRef<AbortController | null>(null);

  const documentQuery = useQuery({
    queryKey: ["document", documentId],
    queryFn: ({ signal }) => {
      const controller = new AbortController();
      abortRef.current?.abort();
      abortRef.current = controller;
      if (signal.aborted) {
        controller.abort();
      }
      const forwardAbort = () => controller.abort();
      signal.addEventListener("abort", forwardAbort, { once: true });
      return apiClient.getDocument(documentId as number, { signal: controller.signal })
        .finally(() => signal.removeEventListener("abort", forwardAbort));
    },
    enabled: documentId !== null,
    retry: false,
  });

  // A different document id, or leaving the reader, aborts the in-flight read so
  // a late response can never settle into the new scope.
  useEffect(() => () => {
    abortRef.current?.abort();
  }, [documentId]);

  const document: DocumentResponse | null = documentQuery.data ?? null;
  const libraryTarget = document ? documentLibraryTarget(document, fromSearch) : null;
  const originUrl = document ? safeExternalUrl(document.source_uri) : null;

  const goBackToSearch = useCallback(() => {
    if (window.history.length > 1) {
      navigate(-1);
      return;
    }
    const session = loadSearchSession();
    if (session?.filters.query) {
      navigate(buildSearchReturnLocation(session.filters, session.page));
      return;
    }
    navigate("/search");
  }, [navigate]);

  const openSource = useCallback(() => {
    if (libraryTarget) {
      navigate(libraryTarget);
      return;
    }
    if (originUrl) {
      window.open(originUrl, "_blank", "noopener,noreferrer");
    }
  }, [libraryTarget, navigate, originUrl]);

  return {
    documentId,
    document,
    fromSearch,
    highlightQuery,
    libraryTarget,
    originUrl,
    isLoading: documentQuery.isLoading,
    isFetching: documentQuery.isFetching,
    error: documentQuery.error,
    errorKind: documentId === null ? "invalid" as const : documentErrorKind(documentQuery.error),
    reload: () => {
      void documentQuery.refetch();
    },
    goBackToSearch,
    openSource,
  };
}
