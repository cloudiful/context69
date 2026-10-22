import { render, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { toast } from "sonner";
import { vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { appRoutes } from "@/app/router";
import { AppToaster } from "@/app/toaster";
import { PreferencesBridge } from "@/features/preferences/preferences-bridge";
import { clearSession, restoreSession } from "@/lib/auth-store";
import { applyTimeZoneSelection, setLocale, setTheme } from "@/lib/preferences-store";
import { openapiClient } from "@/services/openapi-client";
import type {
  DocumentResponse,
  GroupResponse,
  SearchHit,
  SearchPagination,
  SearchResponse,
  SourceStatus,
} from "@/services/api";

export const TEST_USER = {
  user_id: 1,
  login_name: "admin",
  display_name: "Administrator",
  is_admin: true,
  personal_group_path: "personal-admin",
  personal_group_role: "owner" as const,
};

export function searchHit(overrides: Partial<SearchHit> & Pick<SearchHit, "chunk_id" | "title" | "score">): SearchHit {
  const { chunk_id, title, score, ...rest } = overrides;
  return {
    chunk_id,
    chunk_index: 0,
    chunk_text: `${title} body text`,
    document_id: 1,
    external_id: `ext-${chunk_id}`,
    group_key: "group",
    group_path: "personal-admin/default",
    source_key: "gov_documents",
    source_uri: "https://example.test/doc",
    visibility: "private",
    match_reason: "semantic",
    published_at: "2026-10-01T08:00:00Z",
    ...rest,
    title,
    score,
  };
}

export function searchPagination(overrides: Partial<SearchPagination> = {}): SearchPagination {
  return {
    page: 1,
    page_size: 8,
    total: 1,
    total_pages: 1,
    has_more: false,
    ...overrides,
  };
}

export function searchResponse(
  query: string,
  items: SearchHit[],
  pagination: Partial<SearchPagination> = {},
): SearchResponse {
  return {
    query,
    items,
    pagination: searchPagination({ total: items.length, ...pagination }),
  };
}

export const PREVIEW_DOCUMENT: DocumentResponse = {
  document_id: 1,
  external_id: "ext-doc-1",
  group_key: "group",
  group_path: "personal-admin/default",
  record_hash: "hash-1",
  source_key: "gov_documents",
  source_uri: "https://example.test/doc",
  title: "Preview Document",
  summary: "Stored summary for the preview.",
  updated_at: "2026-10-01T08:00:00Z",
  visibility: "private",
  chunks: [
    {
      chunk_id: "chunk-doc-1",
      chunk_index: 0,
      record_hash: "hash-1",
      text: "Stored document body about policy.",
    },
  ],
};

const SOURCE: SourceStatus = {
  base_query: "",
  batch_size: 100,
  connection: "primary",
  connector_type: "postgres_sql",
  display_name: "Government Documents",
  group_key: "group",
  group_path: "personal-admin/default",
  has_database_url: true,
  origin_status: "connected",
  source_key: "gov_documents",
  sync_strategy: "full_scan",
  visibility: "private",
};

const GROUP: GroupResponse = {
  group_id: 1,
  group_key: "group",
  group_path: "personal-admin/default",
  kind: "personal",
  name: "Default",
  visibility: "private",
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-01T08:00:00Z",
};

export interface RecordedSearchCall {
  method: string;
  path: string;
  body: unknown;
  params: unknown;
}

export interface BackendFailure {
  status: number;
  message: string;
}

export interface SearchBackendOptions {
  sources?: SourceStatus[];
  groups?: GroupResponse[];
  document?: DocumentResponse;
  documentError?: BackendFailure;
  post?: SearchResponse | Array<SearchResponse | BackendFailure>;
  postError?: BackendFailure;
  sourcesError?: BackendFailure;
  groupsError?: BackendFailure;
}

function ok<T>(data: T) {
  return Promise.resolve({ data, response: { ok: true, status: 200 } });
}

function failure(status: number, message: string) {
  return Promise.resolve({
    error: { code: "test_error", message },
    response: { ok: false, status },
  });
}

/** In-memory backend for the search page's generated API calls. */
export function installSearchBackend(options: SearchBackendOptions = {}) {
  const calls: RecordedSearchCall[] = [];
  const postQueue: Array<SearchResponse | BackendFailure> = Array.isArray(options.post)
    ? [...options.post]
    : options.post
      ? [options.post]
      : [];

  vi.stubGlobal("fetch", vi.fn(async () => new Response(
    JSON.stringify({ user: TEST_USER }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  )));

  type Init = { body?: unknown; params?: unknown } | undefined;

  function dispatch(method: string, path: string, init: Init) {
    calls.push({ method, path, body: init?.body, params: init?.params });

    if (path === "/v1/sources") {
      if (options.sourcesError) {
        return failure(options.sourcesError.status, options.sourcesError.message);
      }
      return ok({
        items: options.sources ?? [SOURCE],
        pagination: { page: 1, page_size: 100, total: 1, total_pages: 1 },
      });
    }

    if (path === "/v1/groups") {
      if (options.groupsError) {
        return failure(options.groupsError.status, options.groupsError.message);
      }
      return ok({
        items: options.groups ?? [GROUP],
        pagination: { page: 1, page_size: 100, total: 1, total_pages: 1 },
      });
    }

    if (path === "/v1/documents/{document_id}") {
      if (options.documentError) {
        return failure(options.documentError.status, options.documentError.message);
      }
      return ok(options.document ?? PREVIEW_DOCUMENT);
    }

    if (path === "/v1/search" && method === "POST") {
      if (options.postError) {
        return failure(options.postError.status, options.postError.message);
      }
      const next = postQueue.shift();
      if (!next) {
        return failure(500, "no mocked search response");
      }
      if ("status" in next) {
        return failure(next.status, next.message);
      }
      return ok(next);
    }

    return failure(404, `unmocked ${method} ${path}`);
  }

  const spies = {
    GET: vi.spyOn(openapiClient, "GET"),
    POST: vi.spyOn(openapiClient, "POST"),
    PUT: vi.spyOn(openapiClient, "PUT"),
    PATCH: vi.spyOn(openapiClient, "PATCH"),
    DELETE: vi.spyOn(openapiClient, "DELETE"),
  };

  spies.GET.mockImplementation(((path: string, init?: Init) => dispatch("GET", path, init)) as never);
  spies.POST.mockImplementation(((path: string, init?: Init) => dispatch("POST", path, init)) as never);
  spies.PUT.mockImplementation(((path: string, init?: Init) => dispatch("PUT", path, init)) as never);
  spies.PATCH.mockImplementation(((path: string, init?: Init) => dispatch("PATCH", path, init)) as never);
  spies.DELETE.mockImplementation(((path: string, init?: Init) => dispatch("DELETE", path, init)) as never);

  return {
    calls,
    callsFor(method: string, path: string) {
      return calls.filter((call) => call.method === method && call.path === path);
    },
    spies,
  };
}

type FakeListener = (event: { data?: string }) => void;

/** Minimal EventSource stand-in with explicit frame and failure controls. */
export class FakeEventSource {
  static instances: FakeEventSource[] = [];

  readonly url: string;
  readonly withCredentials: boolean;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  private listeners = new Map<string, Set<FakeListener>>();

  constructor(url: string | URL, init?: EventSourceInit) {
    this.url = String(url);
    this.withCredentials = init?.withCredentials === true;
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: FakeListener) {
    const set = this.listeners.get(type) ?? new Set<FakeListener>();
    set.add(listener);
    this.listeners.set(type, set);
  }

  removeEventListener(type: string, listener: FakeListener) {
    this.listeners.get(type)?.delete(listener);
  }

  close() {
    this.closed = true;
  }

  emit(type: string, data?: unknown) {
    const payload = typeof data === "string" ? data : JSON.stringify(data ?? {});
    for (const listener of [...(this.listeners.get(type) ?? [])]) {
      listener({ data: payload });
    }
  }

  failNetwork() {
    this.onerror?.();
  }

  failFrame() {
    this.emit("error");
  }

  listenerCount(type: string): number {
    return this.listeners.get(type)?.size ?? 0;
  }
}

export function installFakeEventSource() {
  FakeEventSource.instances = [];
  vi.stubGlobal("EventSource", FakeEventSource as unknown as typeof EventSource);
}

export function takeEventSource(): FakeEventSource {
  const source = FakeEventSource.instances.at(-1);
  if (!source) {
    throw new Error("No FakeEventSource was created");
  }
  return source;
}

export function installDomPolyfills() {
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.setPointerCapture = () => undefined;
  Element.prototype.releasePointerCapture = () => undefined;
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
}

export function resetPreferences() {
  setLocale("en");
  setTheme("system");
  applyTimeZoneSelection("auto");
}

export function resetAuth() {
  clearSession(null);
}

export async function renderSearch(path = "/search") {
  await restoreSession();
  const router = createMemoryRouter(appRoutes, { initialEntries: [path] });

  render(
    <AppProviders>
      <PreferencesBridge />
      <RouterProvider router={router} />
      <AppToaster />
    </AppProviders>,
  );

  return router;
}

export async function openSearch(path = "/search") {
  const router = await renderSearch(path);
  await waitFor(() => {
    expect(document.querySelector("form")).not.toBeNull();
  });
  return router;
}

export function resetToasts() {
  toast.dismiss();
  for (const element of document.querySelectorAll("[data-sonner-toaster]")) {
    element.remove();
  }
}
