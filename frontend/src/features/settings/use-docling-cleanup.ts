import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { apiClient } from "@/services/api";
import type {
  DoclingCleanupOutcome,
  DoclingCleanupPageResponse,
  DoclingCleanupStatus,
  Pagination,
} from "@/services/api/api-types";

import { summarizeSettingsError } from "./settings-error";

export const DOCLING_CLEANUP_PAGE_SIZE = 20;
export const DOCLING_CLEANUP_PAGE_SIZES = [20, 50, 100] as const;

/**
 * Independent local-cleanup browser over the persisted outbox. Page size and
 * exact filters are server parameters; a newer query invalidates an older
 * response so a late reply cannot overwrite the current selection. Refresh
 * keeps the current filters and page.
 */
export function useDoclingCleanup() {
  const { t } = useTranslation();

  const [page, setPageState] = useState(1);
  const [pageSize, setPageSizeState] = useState(DOCLING_CLEANUP_PAGE_SIZE);
  const [status, setStatusState] = useState<DoclingCleanupStatus | null>(null);
  const [outcome, setOutcomeState] = useState<DoclingCleanupOutcome | null>(null);
  const [remoteTaskId, setRemoteTaskIdState] = useState("");
  const [response, setResponse] = useState<DoclingCleanupPageResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorStatus, setErrorStatus] = useState<number | null>(null);
  const [loadedOnce, setLoadedOnce] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [woken, setWoken] = useState<number | null>(null);
  const revision = useRef(0);

  const loadPage = useCallback(async () => {
    const requestRevision = ++revision.current;
    setLoading(true);
    setError(null);
    setErrorStatus(null);
    try {
      const next = await apiClient.listDoclingCleanup({
        page,
        pageSize,
        status,
        outcome,
        remoteTaskId,
      });
      if (requestRevision !== revision.current) {
        return;
      }
      setResponse(next);
      setLoadedOnce(true);
    } catch (loadError) {
      if (requestRevision !== revision.current) {
        return;
      }
      const summary = summarizeSettingsError(loadError);
      setErrorStatus(summary.status);
      setError(summary.message || t("settings.docling.remoteTasks.loadFailed"));
    } finally {
      if (requestRevision === revision.current) {
        setLoading(false);
      }
    }
  }, [outcome, page, pageSize, remoteTaskId, status, t]);

  // Async work such as a pending wake must refresh the query selected when it
  // finishes, not the closure captured when it started.
  const latestLoadPage = useRef(loadPage);
  latestLoadPage.current = loadPage;

  useEffect(() => {
    void loadPage();
  }, [loadPage]);

  const setPage = useCallback((next: number) => {
    setPageState(Math.max(1, next));
  }, []);

  const setPageSize = useCallback((next: number) => {
    setPageSizeState(next);
    setPageState(1);
  }, []);

  const setStatus = useCallback((next: DoclingCleanupStatus | null) => {
    setStatusState(next);
    setPageState(1);
  }, []);

  const setOutcome = useCallback((next: DoclingCleanupOutcome | null) => {
    setOutcomeState(next);
    setPageState(1);
  }, []);

  const setRemoteTaskId = useCallback((next: string) => {
    setRemoteTaskIdState(next);
    setPageState(1);
  }, []);

  const resetFilters = useCallback(() => {
    setStatusState(null);
    setOutcomeState(null);
    setRemoteTaskIdState("");
    setPageState(1);
  }, []);

  const wakeCleanup = useCallback(async () => {
    if (cleaning) {
      return;
    }
    setCleaning(true);
    try {
      const result = await apiClient.wakeDoclingCleanup();
      setWoken(result.woken);
      await latestLoadPage.current();
    } catch (cause) {
      const summary = summarizeSettingsError(cause);
      setErrorStatus(summary.status);
      setError(summary.message || t("settings.docling.remoteTasks.loadFailed"));
    } finally {
      setCleaning(false);
    }
  }, [cleaning, t]);

  const pagination: Pagination | null = response?.pagination ?? null;
  const hasFilters = status !== null || outcome !== null || remoteTaskId.trim() !== "";

  return {
    cleaning,
    error,
    errorStatus,
    hasFilters,
    items: response?.items ?? [],
    loadedOnce,
    loading,
    outcome,
    page,
    pageSize,
    pagination,
    refresh: loadPage,
    remoteTaskId,
    resetFilters,
    setOutcome,
    setPage,
    setPageSize,
    setRemoteTaskId,
    setStatus,
    status,
    wakeCleanup,
    woken,
  };
}
