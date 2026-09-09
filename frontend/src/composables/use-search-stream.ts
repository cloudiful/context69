import { onBeforeUnmount, ref } from "vue";

import { apiClient, type SearchRequest, type SearchResponse } from "../services/api";
import { resolveApiUrl } from "../services/api/api-core";

type StreamPage = {
  items: SearchResponse["items"];
  pagination: SearchResponse["pagination"];
};

export interface SearchStreamHooks {
  /** Commit a page: called for the SSE `local`/`reranked` pages and for the
   * POST fallback/response so the caller renders exactly one source of truth.
   */
  onResults: (response: SearchResponse) => void;
}

/** Build the GET /v1/search/stream URL for a first-page (cursor-less) request. */
export function buildSearchStreamUrl(payload: SearchRequest): string {
  const params = new URLSearchParams();
  params.set("query", payload.query);
  if (payload.limit != null) params.set("limit", String(payload.limit));
  if (payload.locale) params.set("locale", payload.locale);
  if (payload.source_key) params.set("source_key", payload.source_key);
  if (payload.group_path) params.set("group_path", payload.group_path);
  if (payload.published_after) params.set("published_after", payload.published_after);
  if (payload.published_before) params.set("published_before", payload.published_before);
  if (payload.page && payload.page > 1) params.set("page", String(payload.page));
  if (payload.cursor) params.set("cursor", payload.cursor);
  if (payload.sort && payload.sort !== "relevance") {
    params.set("sort", payload.sort);
  }
  return `${resolveApiUrl("/v1/search/stream")}?${params.toString()}`;
}

const FALLBACK_NOTICE_MS = 6000;

export interface UseSearchStream {
  /** True while no page has been committed yet (spinner state). */
  loading: ReturnType<typeof ref<boolean>>;
  /** True between the `local` page and the `reranked`/`done` frame. */
  refining: ReturnType<typeof ref<boolean>>;
  /** True shortly after the stream failed and results came from POST. */
  fallbackNotice: ReturnType<typeof ref<boolean>>;
  /** Execute a search. Cursor-bearing requests always use POST; fresh
   * first-page requests prefer the SSE stream with a POST fallback. */
  run: (payload: SearchRequest) => Promise<void>;
  /** Cancel the active request/stream (new search or unmount). */
  abort: () => void;
}

export function useSearchStream(hooks: SearchStreamHooks): UseSearchStream {
  const loading = ref(false);
  const refining = ref(false);
  const fallbackNotice = ref(false);

  let runId = 0;
  let stream: EventSource | null = null;
  let streamSettle: ((ok: boolean) => void) | null = null;
  let controller: AbortController | null = null;
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;

  function clearNotice() {
    fallbackNotice.value = false;
    if (noticeTimer != null) {
      clearTimeout(noticeTimer);
      noticeTimer = null;
    }
  }

  function showFallbackNotice() {
    fallbackNotice.value = true;
    if (noticeTimer != null) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => {
      fallbackNotice.value = false;
      noticeTimer = null;
    }, FALLBACK_NOTICE_MS);
  }

  function teardownStream() {
    if (stream) {
      // Detach every handler first so close() cannot fire the error fallback.
      stream.onopen = null;
      stream.onerror = null;
      stream.close();
      stream = null;
    }
  }

  function cancelResources() {
    teardownStream();
    if (controller) {
      controller.abort();
      controller = null;
    }
    // Let an in-flight stream settle so its awaiting `run` can observe the
    // cancellation and bail out on the run-id guard.
    if (streamSettle) {
      const settle = streamSettle;
      streamSettle = null;
      settle(false);
    }
    loading.value = false;
    refining.value = false;
  }

  function cancelActive() {
    runId += 1;
    cancelResources();
  }

  async function runPost(id: number, payload: SearchRequest) {
    controller = new AbortController();
    try {
      const response = await apiClient.search(payload, { signal: controller.signal });
      if (id !== runId) return;
      hooks.onResults(response);
    } catch (error) {
      if (id !== runId) return;
      if (error instanceof Error && error.name === "AbortError") return;
      throw error;
    } finally {
      if (id === runId) {
        loading.value = false;
        refining.value = false;
      }
    }
  }

  /** Resolve `true` when the SSE exchange completed with a `done` frame. */
  function openStream(id: number, payload: SearchRequest): Promise<boolean> {
    return new Promise((resolve) => {
      let es: EventSource;
      let settled = false;
      streamSettle = resolve;
      const finish = (ok: boolean) => {
        if (settled) return;
        settled = true;
        if (streamSettle === resolve) streamSettle = null;
        teardownStream();
        resolve(ok);
      };

      const commitPage = (raw: string) => {
        if (id !== runId) return;
        try {
          const page = JSON.parse(raw) as StreamPage;
          hooks.onResults({
            query: payload.query,
            items: page.items,
            pagination: page.pagination,
          });
          loading.value = false;
        } catch {
          // Ignore malformed frames; the stream error path handles failures.
        }
      };

      try {
        es = new EventSource(buildSearchStreamUrl(payload), { withCredentials: true });
      } catch {
        if (streamSettle === resolve) streamSettle = null;
        resolve(false);
        return;
      }
      stream = es;

      es.addEventListener("local", (event) => {
        if (id !== runId) return;
        commitPage((event as MessageEvent).data);
        // A reranked frame may follow; show the slim indicator meanwhile.
        refining.value = true;
      });
      es.addEventListener("reranked", (event) => {
        if (id !== runId) return;
        commitPage((event as MessageEvent).data);
        refining.value = false;
      });
      es.addEventListener("done", () => {
        if (id !== runId) return;
        refining.value = false;
        finish(true);
      });
      es.addEventListener("error", () => {
        // In-band server error frame: surface through the POST fallback so
        // cursor/ordering rejections keep their HTTP meaning.
        if (id !== runId) return;
        finish(false);
      });
      es.onerror = () => {
        // Network-level failure (or server close without `done`).
        if (id !== runId) return;
        finish(false);
      };
    });
  }

  async function run(payload: SearchRequest) {
    const id = ++runId;
    cancelResources();
    clearNotice();
    loading.value = true;
    refining.value = false;
    controller = null;

    const canStream = typeof EventSource === "function" && !payload.cursor;
    if (!canStream) {
      await runPost(id, payload);
      return;
    }

    const completed = await openStream(id, payload);
    if (id !== runId) return;
    if (completed) {
      loading.value = false;
      refining.value = false;
      return;
    }
    // The stream failed or the server reported an error: fall back to the
    // standard POST endpoint and keep the page consistent. The notice is
    // subtle and non-blocking; errors from the POST request still propagate.
    showFallbackNotice();
    await runPost(id, payload);
  }

  onBeforeUnmount(() => {
    cancelActive();
    clearNotice();
  });

  return {
    loading,
    refining,
    fallbackNotice,
    run,
    abort: cancelActive,
  };
}
