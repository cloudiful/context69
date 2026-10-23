import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { apiClient } from "@/services/api";
import type { DoclingRemoteTask } from "@/services/api/api-types";

import { summarizeSettingsError } from "./settings-error";

export const DOCLING_REMOTE_TASKS_PAGE_SIZE = 20;
export const DOCLING_REMOTE_TASKS_PAGE_SIZES = [20, 50, 100] as const;

/**
 * Read-only Docling remote task browser. Pagination is the upstream offset
 * contract; a page, page-size or newer load invalidates an older response so a
 * late reply cannot overwrite the current window.
 */
export function useDoclingRemoteTasks() {
  const { t } = useTranslation();

  const [items, setItems] = useState<DoclingRemoteTask[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [offset, setOffset] = useState(0);
  const [pageSize, setPageSizeState] = useState(DOCLING_REMOTE_TASKS_PAGE_SIZE);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorStatus, setErrorStatus] = useState<number | null>(null);
  const [loadedOnce, setLoadedOnce] = useState(false);
  const loadRevision = useRef(0);
  const offsetRef = useRef(offset);
  const pageSizeRef = useRef(pageSize);
  offsetRef.current = offset;
  pageSizeRef.current = pageSize;

  const loadPage = useCallback(async () => {
    const revision = ++loadRevision.current;
    const requestOffset = offsetRef.current;
    const requestLimit = pageSizeRef.current;
    setLoading(true);
    setError(null);
    setErrorStatus(null);
    try {
      const response = await apiClient.listDoclingRemoteTasks({
        limit: requestLimit,
        offset: requestOffset,
      });
      if (
        revision !== loadRevision.current
        || requestOffset !== offsetRef.current
        || requestLimit !== pageSizeRef.current
      ) {
        return;
      }
      setItems(response.items);
      setError(response.remote_error ?? null);
      setHasMore(response.has_more);
      setLoadedOnce(true);
    } catch (loadError) {
      if (
        revision !== loadRevision.current
        || requestOffset !== offsetRef.current
        || requestLimit !== pageSizeRef.current
      ) {
        return;
      }
      const summary = summarizeSettingsError(loadError);
      setErrorStatus(summary.status);
      setError(summary.message || t("settings.docling.remoteTasks.loadFailed"));
      setItems([]);
      setHasMore(false);
    } finally {
      if (
        revision === loadRevision.current
        && requestOffset === offsetRef.current
        && requestLimit === pageSizeRef.current
      ) {
        setLoading(false);
      }
    }
  }, [t]);

  useEffect(() => {
    void loadPage();
  }, [loadPage, offset, pageSize]);

  const nextPage = useCallback(() => {
    if (!hasMore || loading) {
      return;
    }
    setOffset((current) => current + pageSize);
  }, [hasMore, loading, pageSize]);

  const prevPage = useCallback(() => {
    if (offset === 0 || loading) {
      return;
    }
    setOffset((current) => Math.max(0, current - pageSize));
  }, [loading, offset, pageSize]);

  const setPageSize = useCallback((next: number) => {
    setPageSizeState(next);
    setOffset(0);
  }, []);

  return {
    error,
    errorStatus,
    hasMore,
    items,
    loadedOnce,
    loading,
    nextPage,
    offset,
    pageSize,
    prevPage,
    refresh: loadPage,
    setPageSize,
  };
}
