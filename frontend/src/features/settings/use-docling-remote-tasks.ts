import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { apiClient } from "@/services/api";
import type { DoclingCleanupTask, DoclingRemoteTask } from "@/services/api/api-types";

import { summarizeSettingsError } from "./settings-error";

export const DOCLING_REMOTE_TASKS_LIMIT = 20;

/**
 * Read-only Docling remote task browser and the explicit stopped-task cleanup
 * wake-up. Pagination is offset based; a page change or a newer load invalidates
 * an older response so a late reply cannot overwrite the current page.
 */
export function useDoclingRemoteTasks() {
  const { t } = useTranslation();

  const [items, setItems] = useState<DoclingRemoteTask[]>([]);
  const [cleanup, setCleanup] = useState<DoclingCleanupTask[]>([]);
  const [cleaning, setCleaning] = useState(false);
  const [cleanupWoken, setCleanupWoken] = useState<number | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [offset, setOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorStatus, setErrorStatus] = useState<number | null>(null);
  const [loadedOnce, setLoadedOnce] = useState(false);
  const loadRevision = useRef(0);
  const offsetRef = useRef(offset);
  offsetRef.current = offset;

  const loadPage = useCallback(async () => {
    const revision = ++loadRevision.current;
    const requestOffset = offsetRef.current;
    setLoading(true);
    setError(null);
    setErrorStatus(null);
    try {
      const response = await apiClient.listDoclingRemoteTasks({
        limit: DOCLING_REMOTE_TASKS_LIMIT,
        offset: requestOffset,
      });
      if (revision !== loadRevision.current || requestOffset !== offsetRef.current) {
        return;
      }
      setItems(response.items);
      setCleanup(response.cleanup ?? []);
      setError(response.remote_error ?? null);
      setHasMore(response.has_more);
      setLoadedOnce(true);
    } catch (loadError) {
      if (revision !== loadRevision.current || requestOffset !== offsetRef.current) {
        return;
      }
      const summary = summarizeSettingsError(loadError);
      setErrorStatus(summary.status);
      setError(summary.message || t("settings.docling.remoteTasks.loadFailed"));
      setItems([]);
      setHasMore(false);
    } finally {
      if (revision === loadRevision.current && requestOffset === offsetRef.current) {
        setLoading(false);
      }
    }
  }, [t]);

  useEffect(() => {
    void loadPage();
  }, [loadPage, offset]);

  const cleanStoppedTasks = useCallback(async () => {
    if (cleaning) {
      return;
    }
    const revision = loadRevision.current;
    setCleaning(true);
    try {
      const response = await apiClient.wakeDoclingCleanup();
      if (revision === loadRevision.current) {
        setCleanupWoken(response.woken);
        await loadPage();
      }
    } catch (cause) {
      if (revision === loadRevision.current) {
        const summary = summarizeSettingsError(cause);
        setError(summary.message || t("settings.docling.remoteTasks.loadFailed"));
        setErrorStatus(summary.status);
      }
    } finally {
      setCleaning(false);
    }
  }, [cleaning, loadPage, t]);

  const nextPage = useCallback(() => {
    if (!hasMore || loading) {
      return;
    }
    setOffset((current) => current + DOCLING_REMOTE_TASKS_LIMIT);
  }, [hasMore, loading]);

  const prevPage = useCallback(() => {
    if (offset === 0 || loading) {
      return;
    }
    setOffset((current) => Math.max(0, current - DOCLING_REMOTE_TASKS_LIMIT));
  }, [loading, offset]);

  return {
    cleanStoppedTasks,
    cleanup,
    cleanupWoken,
    cleaning,
    error,
    errorStatus,
    hasMore,
    items,
    loadedOnce,
    loading,
    nextPage,
    offset,
    prevPage,
    refresh: loadPage,
  };
}
