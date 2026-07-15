import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { setGuest } from "../test-utils/auth";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { installMockStorage } from "../test-utils/storage";
import { LOCALE_STORAGE_KEY } from "../i18n/locale";
import AppSelectField from "../components/AppSelectField.vue";

import SettingsView from "./SettingsView.vue";

const runtimeResponse = {
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
    max_upload_size_mb: 64,
    max_upload_request_size_mb: 128,
    ingest_concurrency: 2,
    pdf_pages_per_task: 5,
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
  },
  vlm: {
    openai_base_url: "https://openrouter.ai/api/v1",
    has_api_key: true,
    vlm_pipeline_model: "gemini-3-flash",
    picture_description_model: "gpt-4o-mini",
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
  state: "idle" as const,
  processed_chunks: 0,
  total_chunks: 0,
};

function createApiSpies() {
  return {
    getRuntimeSettings: vi.spyOn(apiClient, "getRuntimeSettings").mockResolvedValue(runtimeResponse as never),
    updateRuntimeSettings: vi.spyOn(apiClient, "updateRuntimeSettings").mockResolvedValue(runtimeResponse as never),
    testValkeyConnection: vi.spyOn(apiClient, "testValkeyConnection").mockResolvedValue(undefined as never),
    getVectorIndexRebuildStatus: vi.spyOn(apiClient, "getVectorIndexRebuildStatus").mockResolvedValue(vectorRebuildStatus as never),
    startVectorIndexRebuild: vi.spyOn(apiClient, "startVectorIndexRebuild").mockResolvedValue({
      ...vectorRebuildStatus,
      state: "running",
    } as never),
    getDoclingSettings: vi.spyOn(apiClient, "getDoclingSettings").mockResolvedValue(doclingResponse as never),
    getSearchSettings: vi.spyOn(apiClient, "getSearchSettings").mockResolvedValue(searchSettingsResponse as never),
    updateDoclingSettings: vi.spyOn(apiClient, "updateDoclingSettings").mockResolvedValue(doclingResponse as never),
    updateSearchSettings: vi.spyOn(apiClient, "updateSearchSettings").mockResolvedValue({
      ...searchSettingsResponse,
      has_api_key: true,
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

let apiSpies: ReturnType<typeof createApiSpies>;

async function mountSettingsView(path: string, i18n = createTestI18n("en")) {
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
    },
  });

  await flushPromises();

  return { wrapper, router };
}

describe("SettingsView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    installMockStorage();
    setGuest();
    apiSpies = createApiSpies();
  });

  it("loads settings and preserves shared save flow across runtime, docling, and search subpages", async () => {
    const { wrapper, router } = await mountSettingsView("/settings/runtime");

    await vi.waitFor(() => {
      expect(apiSpies.getRuntimeSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.getDoclingSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.getSearchSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.listPersonalAccessTokens).toHaveBeenCalledTimes(1);
      expect(wrapper.find("#runtime-embedding-model").exists()).toBe(true);
    });

    expect(wrapper.get("#runtime-scheduler-valkey-url").attributes("placeholder")).toBe("redis://valkey:6379/0");
    expect(wrapper.get("#runtime-embedding-base-url").element).toBeTruthy();
    expect(wrapper.find("#runtime-embedding-clear-api-key").exists()).toBe(false);
    expect(wrapper.find('[data-testid="runtime-vector-rebuild"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="runtime-file-library-trusted-proxy"]').exists()).toBe(true);
    await wrapper.get('[data-testid="runtime-file-library-trusted-proxy"]').trigger("click");
    await wrapper.get("#runtime-embedding-model").setValue("text-embedding-3-small");

    await router.push("/settings/docling");
    await flushPromises();
    expect(wrapper.find("#docling-base-url").exists()).toBe(true);
    expect(wrapper.find("#docling-clear-api-key").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Stored key");
    expect(wrapper.text()).not.toContain("No key stored");
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");

    await router.push("/settings/search");
    await flushPromises();
    await wrapper.get("#search-rerank-api-key").setValue("rerank-secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledWith(expect.objectContaining({
      embedding: expect.objectContaining({
        base_url: "https://openrouter.ai/api/v1",
        model: "text-embedding-3-small",
      }),
      scheduler: expect.objectContaining({
        valkey_url: "redis://valkey:6379/0",
      }),
      file_library: expect.objectContaining({
        trusted_proxy_enabled: true,
      }),
    }));
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledWith(expect.objectContaining({
      connection: expect.objectContaining({
        base_url: "http://docling.internal:5001",
      }),
      vlm: expect.objectContaining({
        openai_base_url: "https://openrouter.ai/api/v1",
      }),
    }));
    expect(apiSpies.updateSearchSettings).toHaveBeenCalledWith(expect.objectContaining({
      mode: "hybrid",
      rerank_enabled: true,
      api_key: "rerank-secret",
      clear_api_key: false,
    }));
    expect(wrapper.text()).toContain("Settings saved");
  });

  it("switches locale and theme from the settings page", async () => {
    const i18n = createTestI18n("en");
    const { wrapper } = await mountSettingsView("/settings/appearance", i18n);

    const selects = wrapper.findAllComponents(AppSelectField).filter((component) =>
      ["settings-locale-select", "settings-theme-select"].includes(component.props("testId") ?? ""),
    );

    expect(selects).toHaveLength(2);

    await selects[0].vm.$emit("update:modelValue", "zh-CN");
    await selects[1].vm.$emit("update:modelValue", "light");

    expect(i18n.global.locale.value).toBe("zh-CN");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(window.localStorage.getItem("context69.theme")).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("saves direct embedding credentials through the single page save action", async () => {
    const { wrapper } = await mountSettingsView("/settings/runtime");
    await wrapper.get("#runtime-embedding-base-url").setValue("https://embedding.internal/v1");
    await wrapper.get("#runtime-embedding-api-key").setValue("embedding-secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledWith(expect.objectContaining({
      embedding: expect.objectContaining({
        base_url: "https://embedding.internal/v1",
        api_key: "embedding-secret",
      }),
    }));
  });

  it("tests the current Valkey URL without saving settings", async () => {
    const { wrapper } = await mountSettingsView("/settings/runtime");

    await wrapper.get("#runtime-scheduler-valkey-url").setValue(" redis://shared-valkey:6379/2 ");
    await wrapper.get('[data-testid="runtime-valkey-test"]').trigger("click");
    await flushPromises();

    expect(apiSpies.testValkeyConnection).toHaveBeenCalledWith({
      valkey_url: "redis://shared-valkey:6379/2",
    });
    expect(apiSpies.updateRuntimeSettings).not.toHaveBeenCalled();
  });

  it("creates and reveals a personal access token", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });

    const { wrapper } = await mountSettingsView("/settings/access-tokens");

    await wrapper.get("#personal-access-token-name").setValue("CLI");
    const libraryToggle = wrapper.findComponent('[data-testid="pat-scope-library"]') as unknown as {
      vm: { $emit: (event: string, value: boolean) => void };
    };
    libraryToggle.vm.$emit("update:modelValue", true);
    await flushPromises();
    await wrapper.get('[data-testid="personal-access-token-create"]').trigger("click");
    await flushPromises();

    expect(apiSpies.createPersonalAccessToken).toHaveBeenCalledWith({
      name: "CLI",
      scopes: ["search", "library"],
      expires_in_days: 30,
    });
    expect(wrapper.get('[data-testid="personal-access-token-secret"]').text()).toContain("ctx_pat_secret");
  });

  it("requires a token name before allowing personal access token creation", async () => {
    const { wrapper } = await mountSettingsView("/settings/access-tokens");

    const createButton = wrapper.get('[data-testid="personal-access-token-create"]');
    expect(createButton.attributes("disabled")).toBeDefined();
    expect(wrapper.text()).not.toContain("Token name is required.");

    await wrapper.get("#personal-access-token-name").setValue("CLI");
    await flushPromises();

    expect(wrapper.get('[data-testid="personal-access-token-create"]').attributes("disabled")).toBeUndefined();
  });
});
