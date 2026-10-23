import { render, screen, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import { createMemoryRouter, RouterProvider } from "react-router";
import { vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { AppToaster } from "@/app/toaster";
import { PreferencesBridge } from "@/features/preferences/preferences-bridge";
import { appRoutes } from "@/app/router";
import { clearSession, restoreSession } from "@/lib/auth-store";
import { applyTimeZoneSelection, setLocale, setTheme } from "@/lib/preferences-store";
import { openapiClient } from "@/services/openapi-client";
import type {
  AdminUserPageResponse,
  AdminUserResponse,
  CreatePersonalAccessTokenResponse,
  DoclingRemoteTasksResponse,
  DoclingSettingsResponse,
  PersonalAccessTokenPageResponse,
  PersonalAccessTokenResponse,
  RuntimeSettingsResponse,
  SearchSettingsResponse,
  TaskResponse,
  TestLlmProviderResponse,
  TranslationProviderPageResponse,
  TranslationSettingsResponse,
} from "@/services/api";

export interface RecordedCall {
  method: string;
  path: string;
  body: unknown;
  params: unknown;
}

export interface FakeBackendOptions {
  isAdmin?: boolean;
  runtime?: RuntimeSettingsResponse;
  docling?: DoclingSettingsResponse;
  search?: SearchSettingsResponse;
  translation?: TranslationSettingsResponse;
  tokens?: PersonalAccessTokenResponse[];
  users?: AdminUserResponse[];
  vectorTask?: TaskResponse | null;
  remote?: DoclingRemoteTasksResponse;
  fail?: {
    runtimePut?: boolean;
    doclingPut?: boolean;
    searchPut?: boolean;
    translationPut?: boolean;
    testEmbedding?: boolean;
  };
}

export const ADMIN_USER = {
  user_id: 1,
  login_name: "admin",
  display_name: "Administrator",
  is_admin: true,
  personal_group_path: "personal-admin",
  personal_group_role: "owner" as const,
};

export const RUNTIME_SETTINGS: RuntimeSettingsResponse = {
  qdrant: {
    url: "http://qdrant:6333",
    collection_name: "context69",
    recreate_on_dimension_mismatch: false,
  },
  embedding: {
    base_url: "http://embedding:8080/v1",
    dimensions: 768,
    has_api_key: true,
    model: "embed-1",
    timeout_secs: 30,
  },
  scheduler: {
    interval_secs: 300,
    job_id: "context69-sync",
    max_concurrency: 4,
    run_on_start: true,
    valkey_url: "redis://valkey:6379/0",
  },
  chunking: { max_chars: 1200, overlap_chars: 200 },
  file_library: {
    ingest_concurrency: 2,
    max_upload_request_size_mb: 128,
    max_upload_size_mb: 128,
    storage_root: "/data/library",
    trusted_proxy_enabled: false,
    url_import_concurrency: 2,
    url_import_min_interval_ms: 1000,
  },
};

export const DOCLING_SETTINGS: DoclingSettingsResponse = {
  configured: true,
  source: "database",
  connection: {
    base_url: "http://docling:5001",
    max_inflight: 2,
    poll_interval_secs: 2,
    task_timeout_secs: 600,
    timeout_secs: 120,
  },
  vlm: {
    has_api_key: false,
    openai_base_url: null,
    picture_description_model: null,
    picture_description_preset: null,
    vlm_pipeline_model: null,
    code_formula_model: null,
  },
};

export const SEARCH_SETTINGS: SearchSettingsResponse = {
  candidate_limit: 40,
  has_api_key: true,
  keyword_weight: 0.35,
  mode: "hybrid",
  rerank_base_url: "https://openrouter.ai/api/v1",
  rerank_enabled: true,
  rerank_model: "cohere/rerank-4-fast",
  timeout_secs: 10,
  vector_weight: 0.55,
};

export const TRANSLATION_SETTINGS: TranslationSettingsResponse = {
  providers: [
    {
      provider: "deepl",
      enabled: false,
      priority: 0,
      endpoint: null,
      model: null,
      llm_api_kind: null,
      deepl_plan: "free",
      monthly_character_limit: 500_000,
      has_api_key: true,
      current_month_characters: 1200,
    },
    {
      provider: "llm",
      enabled: true,
      priority: 1,
      endpoint: "https://openrouter.ai/api/v1",
      model: "openai/gpt-4.1-mini",
      llm_api_kind: "openai_chat_completions",
      deepl_plan: null,
      monthly_character_limit: null,
      has_api_key: true,
      current_month_characters: 340,
    },
    {
      provider: "libretranslate",
      enabled: false,
      priority: 2,
      endpoint: null,
      model: null,
      llm_api_kind: null,
      deepl_plan: null,
      monthly_character_limit: null,
      has_api_key: false,
      current_month_characters: 0,
    },
  ],
};

export const ACCESS_TOKEN: PersonalAccessTokenResponse = {
  token_id: "11111111-1111-4111-8111-111111111111",
  name: "cli",
  display_prefix: "c69_abc",
  scopes: ["search"],
  created_at: "2026-10-01T08:00:00Z",
  expires_at: "2026-11-01T08:00:00Z",
  last_used_at: null,
  revoked_at: null,
  updated_at: "2026-10-01T08:00:00Z",
};

export const ADMIN_USER_ROW: AdminUserResponse = {
  user_id: 2,
  login_name: "planner",
  display_name: "Planner",
  is_admin: false,
  disabled_at: null,
  created_at: "2026-09-01T08:00:00Z",
  updated_at: "2026-09-01T08:00:00Z",
};

export const VECTOR_TASK: TaskResponse = {
  task_id: "task-vector-1",
  kind: "vector_rebuild",
  status: "succeeded",
  origin: "manual",
  progress: {
    cancelled: 0,
    failed: 0,
    queued: 0,
    running: 0,
    succeeded: 10,
    total: 10,
    waiting: 0,
  },
  created_at: "2026-10-10T08:00:00Z",
  updated_at: "2026-10-10T08:05:00Z",
};

export const REMOTE_TASKS: DoclingRemoteTasksResponse = {
  items: [
    {
      task_id: "remote-1",
      task_type: "convert",
      task_status: "started",
      created_at: "2026-10-10T08:00:00Z",
    },
  ],
  cleanup: [
    {
      id: "22222222-2222-4222-8222-222222222222",
      task_id: "33333333-3333-4333-8333-333333333333",
      remote_task_id: "remote-0",
      status: "pending",
      attempt_count: 0,
      cleanup_deadline_at: "2026-10-10T09:00:00Z",
      next_attempt_at: "2026-10-10T08:05:00Z",
      outcome: null,
      reason_code: "unspecified",
    },
  ],
  has_more: true,
  remote_error: null,
};

function ok<T>(data: T) {
  return Promise.resolve({ data, response: { ok: true, status: 200 } });
}

function failure(status: number, message: string) {
  return Promise.resolve({
    error: { code: "test_error", message },
    response: { ok: false, status },
  });
}

function page(pageNumber: number, pageSize: number, total: number) {
  return {
    page: pageNumber,
    page_size: pageSize,
    total,
    total_pages: Math.max(1, Math.ceil(total / Math.max(1, pageSize))),
  };
}

/**
 * In-memory backend for the settings routes. It spies the generated client so
 * every request body and outcome is assertable, and it keeps the saved state so
 * a save can be followed by the real adoption path.
 */
export function installFakeBackend(options: FakeBackendOptions = {}) {
  const state = {
    runtime: structuredClone(options.runtime ?? RUNTIME_SETTINGS),
    docling: structuredClone(options.docling ?? DOCLING_SETTINGS),
    search: structuredClone(options.search ?? SEARCH_SETTINGS),
    translation: structuredClone(options.translation ?? TRANSLATION_SETTINGS),
    tokens: structuredClone(options.tokens ?? [ACCESS_TOKEN]),
    users: structuredClone(options.users ?? [ADMIN_USER_ROW]),
    vectorTask: options.vectorTask === undefined ? structuredClone(VECTOR_TASK) : options.vectorTask,
    remote: structuredClone(options.remote ?? REMOTE_TASKS),
    isAdmin: options.isAdmin ?? true,
  };
  const calls: RecordedCall[] = [];
  const fail = options.fail ?? {};

  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(
      typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url,
      "http://127.0.0.1:5199",
    );
    if (url.pathname === "/v1/auth/me") {
      return new Response(JSON.stringify({
        user: { ...ADMIN_USER, is_admin: state.isAdmin },
      }), { status: 200, headers: { "Content-Type": "application/json" } });
    }
    if (url.pathname === "/v1/auth/login" || url.pathname === "/v1/auth/logout") {
      return new Response(null, { status: 204 });
    }
    return new Response(JSON.stringify({ code: "not_found", message: `unmocked ${url.pathname}` }), {
      status: 404,
      headers: { "Content-Type": "application/json" },
    });
  }));

  type Init = { body?: unknown; params?: { query?: Record<string, unknown>; path?: Record<string, string> } } | undefined;

  function record(method: string, path: string, init: Init) {
    calls.push({ method, path, body: init?.body, params: init?.params });
  }

  function dispatch(method: string, path: string, init: Init) {
    record(method, path, init);
    const query = init?.params?.query ?? {};
    const body = init?.body as Record<string, unknown> | undefined;

    if (path === "/v1/settings/runtime") {
      if (method === "PUT" && body) {
        if (fail.runtimePut) {
          return failure(500, "runtime rejected");
        }
        const payload = body as unknown as {
          qdrant: RuntimeSettingsResponse["qdrant"];
          embedding: { base_url: string; model: string; dimensions: number; timeout_secs: number; api_key?: string };
          scheduler: RuntimeSettingsResponse["scheduler"];
          chunking: RuntimeSettingsResponse["chunking"];
          file_library: {
            storage_root: string;
            max_upload_size_mb: number;
            max_upload_request_size_mb: number;
            ingest_concurrency: number;
            url_import_concurrency: number;
            url_import_min_interval_ms: number;
            trusted_proxy_enabled: boolean;
            s3?: {
              endpoint: string;
              region: string;
              bucket: string;
              prefix: string;
              path_style: boolean;
              access_key: string;
              secret_key?: string;
            };
          };
        };
        state.runtime = {
          qdrant: payload.qdrant,
          embedding: {
            base_url: payload.embedding.base_url,
            model: payload.embedding.model,
            dimensions: payload.embedding.dimensions,
            timeout_secs: payload.embedding.timeout_secs,
            has_api_key: !!payload.embedding.api_key || state.runtime.embedding.has_api_key,
          },
          scheduler: {
            interval_secs: payload.scheduler.interval_secs,
            job_id: payload.scheduler.job_id,
            max_concurrency: payload.scheduler.max_concurrency,
            run_on_start: payload.scheduler.run_on_start,
            valkey_url: payload.scheduler.valkey_url ?? null,
          },
          chunking: payload.chunking,
          file_library: {
            storage_root: payload.file_library.storage_root,
            max_upload_size_mb: payload.file_library.max_upload_size_mb,
            max_upload_request_size_mb: payload.file_library.max_upload_request_size_mb,
            ingest_concurrency: payload.file_library.ingest_concurrency,
            url_import_concurrency: payload.file_library.url_import_concurrency,
            url_import_min_interval_ms: payload.file_library.url_import_min_interval_ms,
            trusted_proxy_enabled: payload.file_library.trusted_proxy_enabled,
            s3: payload.file_library.s3
              ? {
                endpoint: payload.file_library.s3.endpoint,
                region: payload.file_library.s3.region,
                bucket: payload.file_library.s3.bucket,
                prefix: payload.file_library.s3.prefix,
                path_style: payload.file_library.s3.path_style,
                access_key: payload.file_library.s3.access_key,
                has_secret_key: !!payload.file_library.s3.secret_key,
              }
              : undefined,
          },
        };
        return ok(state.runtime);
      }
      return ok(state.runtime);
    }

    if (path === "/v1/settings/docling") {
      if (method === "PUT" && body) {
        if (fail.doclingPut) {
          return failure(500, "docling rejected");
        }
        const payload = body as unknown as {
          connection: DoclingSettingsResponse["connection"];
          vlm?: {
            openai_base_url?: string;
            api_key?: string;
            vlm_pipeline_model?: string;
            picture_description_model?: string;
            picture_description_preset?: string;
            code_formula_model?: string;
          };
        };
        state.docling = {
          configured: true,
          source: "database",
          connection: payload.connection,
          vlm: {
            openai_base_url: payload.vlm?.openai_base_url ?? null,
            vlm_pipeline_model: payload.vlm?.vlm_pipeline_model ?? null,
            picture_description_model: payload.vlm?.picture_description_model ?? null,
            picture_description_preset: payload.vlm?.picture_description_preset ?? null,
            code_formula_model: payload.vlm?.code_formula_model ?? null,
            has_api_key: !!payload.vlm?.api_key || state.docling.vlm.has_api_key,
          },
        };
        return ok(state.docling);
      }
      return ok(state.docling);
    }

    if (path === "/v1/settings/search") {
      if (method === "PUT" && body) {
        if (fail.searchPut) {
          return failure(500, "search rejected");
        }
        const payload = body as unknown as {
          mode: SearchSettingsResponse["mode"];
          rerank_enabled: boolean;
          rerank_base_url: string;
          rerank_model: string;
          candidate_limit: number;
          timeout_secs: number;
          vector_weight?: number;
          keyword_weight?: number;
          api_key?: { op: "keep" | "clear" | "set"; value?: string };
        };
        state.search = {
          mode: payload.mode,
          rerank_enabled: payload.rerank_enabled,
          rerank_base_url: payload.rerank_base_url,
          rerank_model: payload.rerank_model,
          candidate_limit: payload.candidate_limit,
          timeout_secs: payload.timeout_secs,
          vector_weight: payload.vector_weight ?? 0,
          keyword_weight: payload.keyword_weight ?? 0,
          has_api_key: payload.api_key?.op === "set"
            ? true
            : payload.api_key?.op === "clear"
              ? false
              : state.search.has_api_key,
        };
        return ok(state.search);
      }
      return ok(state.search);
    }

    if (path === "/v1/settings/translation") {
      if (method === "PUT" && body) {
        if (fail.translationPut) {
          return failure(500, "translation rejected");
        }
        const payload = body as unknown as {
          providers: {
            provider: string;
            enabled?: boolean;
            priority: number;
            endpoint?: string | null;
            api_key?: string | null;
            model?: string | null;
            llm_api_kind?: TranslationSettingsResponse["providers"][number]["llm_api_kind"];
            deepl_plan?: TranslationSettingsResponse["providers"][number]["deepl_plan"];
            monthly_character_limit?: number | null;
          }[];
        };
        const previous = new Map(state.translation.providers.map((item) => [item.provider, item]));
        state.translation = {
          providers: payload.providers.map((provider) => {
            const before = previous.get(provider.provider as typeof state.translation.providers[number]["provider"]);
            return {
              provider: provider.provider as typeof state.translation.providers[number]["provider"],
              enabled: !!provider.enabled,
              priority: provider.priority,
              endpoint: provider.endpoint ?? null,
              model: provider.model ?? null,
              llm_api_kind: provider.llm_api_kind ?? null,
              deepl_plan: provider.deepl_plan ?? null,
              monthly_character_limit: provider.monthly_character_limit ?? null,
              has_api_key: !!provider.api_key || !!before?.has_api_key,
              current_month_characters: before?.current_month_characters ?? 0,
            };
          }),
        };
        return ok(state.translation);
      }
      return ok(state.translation);
    }

    if (path === "/v1/settings/translation/providers") {
      const pageNumber = Number(query.page ?? 1);
      const pageSize = Number(query.page_size ?? 50);
      const response: TranslationProviderPageResponse = {
        items: state.translation.providers.slice((pageNumber - 1) * pageSize, pageNumber * pageSize),
        pagination: page(pageNumber, pageSize, state.translation.providers.length),
      };
      return ok(response);
    }

    if (path === "/v1/settings/translation/providers/llm/test") {
      const result: TestLlmProviderResponse = {
        outcome: "success",
        http_status: 200,
        latency_ms: 42,
        llm_api_kind: "openai_chat_completions",
        safe_endpoint: "https://openrouter.ai/api/v1",
      };
      return ok(result);
    }

    if (path.endsWith("/embedding/test") && fail.testEmbedding) {
      return failure(502, "embedding refused");
    }

    if (path === "/v1/settings/runtime/vector-index/rebuild") {
      state.vectorTask = { ...VECTOR_TASK, status: "running" };
      return ok({ task_id: VECTOR_TASK.task_id });
    }

    if (path.startsWith("/v1/settings/runtime/") || path === "/v1/settings/docling/test") {
      return ok({});
    }

    if (path === "/v1/tasks") {
      return ok({
        items: state.vectorTask ? [state.vectorTask] : [],
        pagination: page(1, 1, state.vectorTask ? 1 : 0),
      });
    }

    if (path.startsWith("/v1/tasks/")) {
      return ok(state.vectorTask);
    }

    if (path === "/v1/admin/docling/tasks") {
      const offset = Number(query.offset ?? 0);
      return ok(offset === 0 ? state.remote : { ...state.remote, items: [], has_more: false });
    }

    if (path === "/v1/admin/docling/cleanup") {
      return ok({ woken: 2 });
    }

    if (path === "/v1/auth/personal-access-tokens") {
      if (method === "POST" && body) {
        const payload = body as unknown as {
          name: string;
          scopes: PersonalAccessTokenResponse["scopes"];
          expires_in_days: number;
        };
        const created: PersonalAccessTokenResponse = {
          ...ACCESS_TOKEN,
          token_id: "44444444-4444-4444-8444-444444444444",
          name: payload.name,
          scopes: payload.scopes,
          expires_at: new Date(Date.now() + payload.expires_in_days * 86_400_000).toISOString(),
        };
        state.tokens = [created, ...state.tokens];
        const response: CreatePersonalAccessTokenResponse = {
          access_token: "c69_revealed_once",
          token: created,
        };
        return ok(response);
      }
      const pageNumber = Number(query.page ?? 1);
      const pageSize = Number(query.page_size ?? 50);
      const response: PersonalAccessTokenPageResponse = {
        items: state.tokens,
        pagination: page(pageNumber, pageSize, state.tokens.length),
      };
      return ok(response);
    }

    if (path.startsWith("/v1/auth/personal-access-tokens/")) {
      const tokenId = init?.params?.path?.token_id ?? path.split("/").at(-1);
      state.tokens = state.tokens.map((token) => (
        token.token_id === tokenId ? { ...token, revoked_at: "2026-10-21T08:00:00Z" } : token
      ));
      return ok(undefined);
    }

    if (path === "/v1/admin/users") {
      if (method === "POST" && body) {
        const payload = body as unknown as { login_name: string; display_name: string; is_admin: boolean };
        const created: AdminUserResponse = {
          ...ADMIN_USER_ROW,
          user_id: 9,
          login_name: payload.login_name,
          display_name: payload.display_name,
          is_admin: payload.is_admin,
        };
        state.users = [...state.users, created];
        return ok(created);
      }
      const pageNumber = Number(query.page ?? 1);
      const pageSize = Number(query.page_size ?? 50);
      const needle = String(query.query ?? "").toLowerCase();
      const filtered = state.users.filter((user) => (
        !needle || user.login_name.includes(needle) || user.display_name.toLowerCase().includes(needle)
      ));
      const response: AdminUserPageResponse = {
        items: filtered,
        pagination: page(pageNumber, pageSize, filtered.length),
      };
      return ok(response);
    }

    if (path.startsWith("/v1/admin/users/")) {
      const segments = path.split("/");
      const loginName = init?.params?.path?.login_name ?? decodeURIComponent(segments[4] ?? "");
      const action = segments[5];
      const target = state.users.find((user) => user.login_name === loginName);
      if (!target) {
        return failure(404, "missing user");
      }
      if (method === "PATCH" && body) {
        const payload = body as unknown as { display_name?: string; is_admin?: boolean };
        state.users = state.users.map((user) => (
          user.login_name === loginName
            ? { ...user, display_name: payload.display_name ?? user.display_name, is_admin: payload.is_admin ?? user.is_admin }
            : user
        ));
        return ok(state.users.find((user) => user.login_name === loginName));
      }
      if (action === "reset-password") {
        return ok(undefined);
      }
      if (action === "disable") {
        state.users = state.users.map((user) => (
          user.login_name === loginName ? { ...user, disabled_at: "2026-10-21T08:00:00Z" } : user
        ));
        return ok(undefined);
      }
      if (action === "enable") {
        state.users = state.users.map((user) => (
          user.login_name === loginName ? { ...user, disabled_at: null } : user
        ));
        return ok(undefined);
      }
    }

    return failure(404, `unmocked ${method} ${path}`);
  }

  const spies = {
    GET: vi.spyOn(openapiClient, "GET").mockImplementation(((path: string, init?: Init) => (
      dispatch("GET", path, init)
    )) as unknown as typeof openapiClient.GET),
    PUT: vi.spyOn(openapiClient, "PUT").mockImplementation(((path: string, init?: Init) => (
      dispatch("PUT", path, init)
    )) as unknown as typeof openapiClient.PUT),
    POST: vi.spyOn(openapiClient, "POST").mockImplementation(((path: string, init?: Init) => (
      dispatch("POST", path, init)
    )) as unknown as typeof openapiClient.POST),
    PATCH: vi.spyOn(openapiClient, "PATCH").mockImplementation(((path: string, init?: Init) => (
      dispatch("PATCH", path, init)
    )) as unknown as typeof openapiClient.PATCH),
    DELETE: vi.spyOn(openapiClient, "DELETE").mockImplementation(((path: string, init?: Init) => (
      dispatch("DELETE", path, init)
    )) as unknown as typeof openapiClient.DELETE),
  };

  return {
    calls,
    spies,
    state,
    callsFor(method: string, path: string) {
      return calls.filter((call) => call.method === method && call.path === path);
    },
  };
}

/**
 * jsdom lacks the pointer-capture and resize APIs Radix and cmdk use; the
 * settings tests exercise the real shadcn controls, so they install these
 * minimal stand-ins locally instead of changing the shared test setup.
 */
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

/** Clears toasts and their portal, which RTL cleanup does not own. */
export function resetToasts() {
  toast.dismiss();
  for (const element of document.querySelectorAll("[data-sonner-toaster]")) {
    element.remove();
  }
}

/** Resets the shared preference stores to their defaults between tests. */
export function resetPreferences() {
  setLocale("en");
  setTheme("system");
  applyTimeZoneSelection("auto");
}

export async function renderSettings(path: string) {
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

export async function openSettings(path: string) {
  const router = await renderSettings(path);
  await waitFor(() => {
    expect(screen.queryByText("Loading runtime settings and defaults.")).not.toBeInTheDocument();
  });
  return router;
}

export function resetAuth() {
  clearSession(null);
}
