/**
 * Shared harness for the settings view tests.
 *
 * The view's own test files and the save-flow test file both mount the same
 * component against the same API surface, so the fixtures, the API spies, the
 * mount and the captured error-toast seam live here once instead of being
 * copied per file.
 *
 * The view itself is imported dynamically inside `mountSettingsView`: the error
 * toast is captured with `vi.mock`, and the mock has to be registered before the
 * module that calls it is resolved, which a static import at the top of this file
 * would do too early.
 */
import { flushPromises, mount } from "@vue/test-utils";
import { vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { setGuest } from "../test-utils/auth";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { installMockStorage } from "../test-utils/storage";

const errorToast = vi.hoisted(() => ({ shown: [] as unknown[] }));

/** The messages the settings page handed the shared error toast. */
export function surfacedReasons(): string[] {
  return (errorToast.shown as { error: unknown; fallback: string }[]).map((entry) =>
    entry.error instanceof Error ? entry.error.message : String(entry.error),
  );
}
vi.mock("../composables/use-error-toast", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../composables/use-error-toast")>();
  return {
    ...actual,
    // Capture what the settings page hands the shared error toast, so the
    // surfaced reason can be asserted without replacing the toast component.
    // The real composable is resolved where the settings page calls it (at the
    // top of setup), and only its returned callback is wrapped, so the wrapped
    // callback never calls a Vue setup composable outside setup.
    useErrorToast: () => {
      const showRealErrorToast = actual.useErrorToast();
      return (error: unknown, fallback: string) => {
        errorToast.shown.push({ error, fallback });
        return showRealErrorToast(error, fallback);
      };
    },
  };
});

/** The runtime draft every case starts from; the save-flow cases resolve it directly. */
export const runtimeResponse = {
  qdrant: {
    url: "http://qdrant:6334",
    collection_name: "context69",
    recreate_on_dimension_mismatch: false,
  },
  embedding: {
    base_url: "https://openrouter.ai/api/v1",
    has_api_key: true,
    model: "text-embedding-3-large",
    dimensions: 3072,
    timeout_secs: 30,
  },
  scheduler: {
    interval_secs: 300,
    run_on_start: true,
    max_concurrency: 4,
    job_id: "context69-sync",
    valkey_url: "redis://valkey:6379/0",
  },
  chunking: {
    max_chars: 1200,
    overlap_chars: 200,
  },
  file_library: {
    storage_root: "/tmp/library",
    max_upload_size_mb: 128,
    max_upload_request_size_mb: 128,
    ingest_concurrency: 2,
    url_import_concurrency: 2,
    url_import_min_interval_ms: 1000,
    trusted_proxy_enabled: false,
  },
};

const doclingResponse = {
  configured: true,
  source: "database",
  connection: {
    base_url: "http://docling:5001",
    timeout_secs: 120,
    poll_interval_secs: 2,
    task_timeout_secs: 600,
    max_inflight: 2,
  },
  vlm: {
    openai_base_url: "https://openrouter.ai/api/v1",
    has_api_key: true,
    vlm_pipeline_model: "gemini-3-flash",
    picture_description_model: "gpt-4o-mini",
    picture_description_preset: "granite_vision",
    code_formula_model: "gpt-4o-mini",
  },
};

const searchSettingsResponse = {
  mode: "hybrid" as const,
  rerank_enabled: true,
  rerank_base_url: "https://openrouter.ai/api/v1",
  rerank_model: "cohere/rerank-4-fast",
  candidate_limit: 40,
  timeout_secs: 10,
  has_api_key: false,
  vector_weight: 0.55,
  keyword_weight: 0.35,
};

const personalAccessTokensResponse = [
  {
    token_id: "00000000-0000-0000-0000-000000000001",
    name: "CLI",
    display_prefix: "ctx_pat_abcd",
    scopes: ["search", "library"],
    expires_at: "2026-12-31T00:00:00Z",
    last_used_at: null,
    revoked_at: null,
    created_at: "2026-06-01T00:00:00Z",
    updated_at: "2026-06-01T00:00:00Z",
  },
];

const vectorRebuildStatus = {
  task_id: "00000000-0000-0000-0000-000000000001",
  kind: "vector_rebuild" as const,
  status: "succeeded" as const,
  group_path: null,
  source_key: null,
  stage: "finalize",
  waiting_reason: null,
  dependency_key: null,
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  eta_seconds: null,
  created_at: "2026-08-02T00:00:00Z",
  started_at: "2026-08-02T00:00:00Z",
  finished_at: "2026-08-02T00:00:00Z",
  updated_at: "2026-08-02T00:00:00Z",
};

function createApiSpies() {
  return {
    getRuntimeSettings: vi.spyOn(apiClient, "getRuntimeSettings").mockResolvedValue(runtimeResponse as never),
    updateRuntimeSettings: vi.spyOn(apiClient, "updateRuntimeSettings").mockResolvedValue(runtimeResponse as never),
    testValkeyConnection: vi.spyOn(apiClient, "testValkeyConnection").mockResolvedValue(undefined as never),
    testEmbeddingConnection: vi.spyOn(apiClient, "testEmbeddingConnection").mockResolvedValue(undefined as never),
    listTasks: vi.spyOn(apiClient, "listTasks").mockResolvedValue({ items: [vectorRebuildStatus], pagination: { page: 1, page_size: 1, total: 1, total_pages: 1 } } as never),
    submitVectorIndexRebuild: vi.spyOn(apiClient, "submitVectorIndexRebuild").mockResolvedValue({ task_id: vectorRebuildStatus.task_id, item_ids: [] } as never),
    getTask: vi.spyOn(apiClient, "getTask").mockResolvedValue(vectorRebuildStatus as never),
    getDoclingSettings: vi.spyOn(apiClient, "getDoclingSettings").mockResolvedValue(doclingResponse as never),
    getSearchSettings: vi.spyOn(apiClient, "getSearchSettings").mockResolvedValue(searchSettingsResponse as never),
    updateDoclingSettings: vi.spyOn(apiClient, "updateDoclingSettings").mockResolvedValue(doclingResponse as never),
    updateSearchSettings: vi.spyOn(apiClient, "updateSearchSettings").mockResolvedValue({
      ...searchSettingsResponse,
      has_api_key: true,
    } as never),
    updateTranslationSettings: vi.spyOn(apiClient, "updateTranslationSettings").mockResolvedValue({
      providers: [],
    } as never),
    listPersonalAccessTokens: vi.spyOn(apiClient, "listPersonalAccessTokens").mockResolvedValue(personalAccessTokensResponse as never),
    createPersonalAccessToken: vi.spyOn(apiClient, "createPersonalAccessToken").mockResolvedValue({
      access_token: "ctx_pat_secret",
      token: personalAccessTokensResponse[0],
    } as never),
    revokePersonalAccessToken: vi.spyOn(apiClient, "revokePersonalAccessToken").mockResolvedValue(undefined as never),
    listAdminUsers: vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue([] as never),
    createAdminUser: vi.spyOn(apiClient, "createAdminUser").mockResolvedValue(undefined as never),
    updateAdminUser: vi.spyOn(apiClient, "updateAdminUser").mockResolvedValue(undefined as never),
    resetAdminUserPassword: vi.spyOn(apiClient, "resetAdminUserPassword").mockResolvedValue(undefined as never),
    disableAdminUser: vi.spyOn(apiClient, "disableAdminUser").mockResolvedValue(undefined as never),
    enableAdminUser: vi.spyOn(apiClient, "enableAdminUser").mockResolvedValue(undefined as never),
  };
}

export type SettingsApiSpies = ReturnType<typeof createApiSpies>;

/** Restores the spies, installs a guest session and returns the fresh spies. */
export function installSettingsViewSpies(): SettingsApiSpies {
  vi.restoreAllMocks();
  installMockStorage();
  setGuest();
  const apiSpies = createApiSpies();
  errorToast.shown.length = 0;
  return apiSpies;
}

export async function mountSettingsView(path: string, i18n = createTestI18n("en")) {
  const { default: SettingsView } = await import("./SettingsView.vue");
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/settings/appearance", name: "settings-appearance", component: SettingsView },
      { path: "/settings/access-tokens", name: "settings-access-tokens", component: SettingsView },
      { path: "/settings/search", name: "settings-search", component: SettingsView },
      { path: "/settings/runtime", name: "settings-runtime", component: SettingsView },
      { path: "/settings/docling", name: "settings-docling", component: SettingsView },
      { path: "/settings/admin-users", name: "settings-admin-users", component: SettingsView },
    ],
  });

  router.push(path);
  await router.isReady();

  const wrapper = mount(SettingsView, {
    attachTo: document.body,
    global: {
      plugins: [testNuxtUiPlugin, i18n, router],
      stubs: {
        UTooltip: {
          template: "<div><slot /><div class='tooltip-stub'>{{ $attrs.text }}</div></div>",
        },
      },
    },
  });

  await flushPromises();

  return { wrapper, router };
}