import { useCallback, useEffect, useRef, useState } from "react";

import type { paths } from "@/generated/openapi";
import { apiClient } from "@/services/api";
import {
  ApiError,
  resolveApiUrl,
} from "@/services/api/api-core";
import type {
  SearchHit,
  SearchPagination,
  SearchRequest,
  SearchResponse,
} from "@/services/api";

/** One committed page: exactly the generated response's page fields. */
export type SearchPage = Pick<SearchResponse, "items" | "pagination">;

type SearchStreamQuery = NonNullable<
  paths["/v1/search/stream"]["get"]["parameters"]["query"]
>;

export const SEARCH_FALLBACK_NOTICE_MS = 6000;

/** Build the GET /v1/search/stream URL for a first-page (cursor-less) request. */
export function buildSearchStreamUrl(payload: SearchRequest): string {
  const query: SearchStreamQuery = { query: payload.query };
  if (payload.limit != null) query.limit = payload.limit;
  if (payload.locale) query.locale = payload.locale;
  if (payload.source_key) query.source_key = payload.source_key;
  if (payload.group_path) query.group_path = payload.group_path;
  if (payload.published_after) query.published_after = payload.published_after;
  if (payload.published_before) query.published_before = payload.published_before;
  if (payload.cursor) query.cursor = payload.cursor;
  if (payload.sort && payload.sort !== "relevance") query.sort = payload.sort;

  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== null && value !== "") {
      params.set(key, String(value));
    }
  }
  return `${resolveApiUrl("/v1/search/stream")}?${params.toString()}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isSearchHit(value: unknown): value is SearchHit {
  if (!isRecord(value)) {
    return false;
  }
  return typeof value.chunk_id === "string"
    && typeof value.document_id === "number"
    && typeof value.title === "string"
    && typeof value.chunk_text === "string"
    && typeof value.score === "number";
}

function isSearchPagination(value: unknown): value is SearchPagination {
  if (!isRecord(value)) {
    return false;
  }
  return typeof value.page === "number"
    && typeof value.page_size === "number"
    && typeof value.total === "number"
    && typeof value.total_pages === "number";
}

/**
 * A frame payload is untrusted JSON: it is validated for the required page
 * shape before use, and anything else triggers the POST fallback instead of a
 * partial render or a crash.
 */
export function parseStreamPage(raw: string): SearchPage | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) {
    return null;
  }
  const items = parsed.items;
  const pagination = parsed.pagination;
  if (!Array.isArray(items) || !items.every(isSearchHit) || !isSearchPagination(pagination)) {
    return null;
  }
  return { items, pagination };
}

export interface SearchStreamController {
  /** True while no page has been committed yet (spinner state). */
  loading: boolean;
  /** True between the `local` page and the `reranked`/`done` frame. */
  refining: boolean;
  /** True shortly after the stream failed and results came from POST. */
  fallbackNotice: boolean;
  /** Execute a search. Cursor-bearing requests always use POST. */
  run: (payload: SearchRequest) => Promise<void>;
  /** Cancel the active request/stream (new search, cancel button, unmount). */
  abort: () => void;
}

/**
 * React controller for the streaming search channel. It owns the request
 * generation, the native EventSource lifecycle, the POST fallback and the
 * notice timer, so no frame, page or error from an older run can replace the
 * current page.
 */
export function useSearchStream(
  onPage: (page: SearchPage) => void,
  options: { noticeMs?: number } = {},
): SearchStreamController {
  const noticeMs = options.noticeMs ?? SEARCH_FALLBACK_NOTICE_MS;
  const onPageRef = useRef(onPage);
  onPageRef.current = onPage;

  const [loading, setLoading] = useState(false);
  const [refining, setRefining] = useState(false);
  const [fallbackNotice, setFallbackNotice] = useState(false);

  const runIdRef = useRef(0);
  const streamRef = useRef<EventSource | null>(null);
  const listenersRef = useRef<Array<[string, (event: Event) => void]>>([]);
  const streamSettleRef = useRef<((completed: boolean) => void) | null>(null);
  const controllerRef = useRef<AbortController | null>(null);
  const noticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearNoticeTimer = useCallback(() => {
    if (noticeTimerRef.current != null) {
      clearTimeout(noticeTimerRef.current);
      noticeTimerRef.current = null;
    }
  }, []);

  const clearNotice = useCallback(() => {
    clearNoticeTimer();
    setFallbackNotice(false);
  }, [clearNoticeTimer]);

  const showFallbackNotice = useCallback(() => {
    setFallbackNotice(true);
    clearNoticeTimer();
    noticeTimerRef.current = setTimeout(() => {
      setFallbackNotice(false);
      noticeTimerRef.current = null;
    }, noticeMs);
  }, [clearNoticeTimer, noticeMs]);

  const teardownStream = useCallback(() => {
    const stream = streamRef.current;
    // Detach every handler first so close() cannot fire the error fallback.
    for (const [type, listener] of listenersRef.current) {
      stream?.removeEventListener(type, listener);
    }
    listenersRef.current = [];
    if (!stream) {
      return;
    }
    stream.onopen = null;
    stream.onerror = null;
    stream.close();
    streamRef.current = null;
  }, []);

  const cancelResources = useCallback(() => {
    teardownStream();
    if (controllerRef.current) {
      controllerRef.current.abort();
      controllerRef.current = null;
    }
    const settle = streamSettleRef.current;
    streamSettleRef.current = null;
    settle?.(false);
    setLoading(false);
    setRefining(false);
  }, [teardownStream]);

  const abort = useCallback(() => {
    runIdRef.current += 1;
    cancelResources();
    clearNotice();
  }, [cancelResources, clearNotice]);

  const runPost = useCallback(async (id: number, payload: SearchRequest) => {
    const controller = new AbortController();
    controllerRef.current = controller;

    try {
      const response = await apiClient.search(payload, { signal: controller.signal });
      if (id !== runIdRef.current) {
        return;
      }
      onPageRef.current({ items: response.items, pagination: response.pagination });
    } catch (error) {
      if (id !== runIdRef.current) {
        return;
      }
      if (error instanceof Error && error.name === "AbortError") {
        return;
      }
      throw error;
    } finally {
      if (id === runIdRef.current) {
        setLoading(false);
        setRefining(false);
      }
    }
  }, []);

  const openStream = useCallback((id: number, payload: SearchRequest): Promise<boolean> => (
    new Promise<boolean>((resolve) => {
      let settled = false;
      const finish = (completed: boolean) => {
        if (settled) {
          return;
        }
        settled = true;
        if (streamSettleRef.current === resolve) {
          streamSettleRef.current = null;
        }
        teardownStream();
        resolve(completed);
      };

      const commitPage = (raw: string): boolean => {
        if (id !== runIdRef.current) {
          return false;
        }
        const page = parseStreamPage(raw);
        if (!page) {
          finish(false);
          return false;
        }
        onPageRef.current(page);
        setLoading(false);
        return true;
      };

      streamSettleRef.current = resolve;

      let source: EventSource;
      try {
        source = new EventSource(buildSearchStreamUrl(payload), { withCredentials: true });
      } catch {
        streamSettleRef.current = null;
        resolve(false);
        return;
      }
      streamRef.current = source;

      const onLocal = (event: Event) => {
        if (id !== runIdRef.current) {
          return;
        }
        if (!commitPage((event as MessageEvent).data)) {
          return;
        }
        // A reranked frame may follow; show the slim indicator meanwhile.
        setRefining(true);
      };

      const onReranked = (event: Event) => {
        if (id !== runIdRef.current) {
          return;
        }
        if (!commitPage((event as MessageEvent).data)) {
          return;
        }
        setRefining(false);
      };

      const onDone = () => {
        if (id !== runIdRef.current) {
          return;
        }
        setRefining(false);
        finish(true);
      };

      const onErrorFrame = () => {
        // In-band server error frame: surface through the POST fallback so
        // cursor/ordering rejections keep their HTTP meaning.
        if (id !== runIdRef.current) {
          return;
        }
        finish(false);
      };

      listenersRef.current = [
        ["local", onLocal],
        ["reranked", onReranked],
        ["done", onDone],
        ["error", onErrorFrame],
      ];
      source.addEventListener("local", onLocal);
      source.addEventListener("reranked", onReranked);
      source.addEventListener("done", onDone);
      source.addEventListener("error", onErrorFrame);

      source.onerror = () => {
        // Network failure, or the server closed without a `done` frame.
        if (id !== runIdRef.current) {
          return;
        }
        finish(false);
      };
    })
  ), [teardownStream]);

  const run = useCallback(async (payload: SearchRequest) => {
    const id = ++runIdRef.current;
    cancelResources();
    clearNotice();
    setLoading(true);
    setRefining(false);

    // Only a cursor-less first page is expressible on the stream endpoint;
    // later windows must use the POST body that carries the page number.
    const canStream = typeof EventSource === "function"
      && !payload.cursor
      && (payload.page ?? 1) === 1;
    if (!canStream) {
      await runPost(id, payload);
      return;
    }

    const completed = await openStream(id, payload);
    if (id !== runIdRef.current) {
      return;
    }
    if (completed) {
      setLoading(false);
      setRefining(false);
      return;
    }

    // The stream failed or reported an in-band error: fall back to the standard
    // POST endpoint and keep the page consistent. The notice is subtle and
    // non-blocking; POST errors still reach the caller.
    showFallbackNotice();
    await runPost(id, payload);
  }, [cancelResources, clearNotice, openStream, runPost, showFallbackNotice]);

  useEffect(() => () => {
    runIdRef.current += 1;
    const stream = streamRef.current;
    for (const [type, listener] of listenersRef.current) {
      stream?.removeEventListener(type, listener);
    }
    listenersRef.current = [];
    if (stream) {
      stream.onopen = null;
      stream.onerror = null;
      stream.close();
      streamRef.current = null;
    }
    controllerRef.current?.abort();
    controllerRef.current = null;
    const settle = streamSettleRef.current;
    streamSettleRef.current = null;
    settle?.(false);
    if (noticeTimerRef.current != null) {
      clearTimeout(noticeTimerRef.current);
      noticeTimerRef.current = null;
    }
  }, []);

  return { loading, refining, fallbackNotice, run, abort };
}

export function isCursorOrderingError(error: unknown): boolean {
  return error instanceof ApiError && error.status === 400 && /cursor/i.test(error.message);
}
