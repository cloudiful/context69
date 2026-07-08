import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Select from "primevue/select";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { setGuest } from "../test-utils/auth";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { installMockStorage } from "../test-utils/storage";
import { LOCALE_STORAGE_KEY } from "../i18n/locale";
import { SEARCH_HISTORY_STORAGE_KEY } from "../utils/search-history";

import SettingsView from "./SettingsView.vue";

const runtimeResponse = {
  qdrant: {
    url: "http://qdrant:6334",
    collection_name: "context69",
    recreate_on_dimension_mismatch: false,
  },
  embedding: {
    provider_account_key: "openrouter-default",
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
  },
};

const doclingResponse = {
  configured: true,
  source: "database",
  connection: {
    base_url: "http://docling:5001",
    timeout_secs: 120,
    poll_interval_secs: 2,
  },
  vlm: {
    provider_account_key: "openrouter-default",
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

const providerAccountsResponse = [
  {
    account_key: "openrouter-default",
    provider_kind: "openai_compatible",
    display_name: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    has_api_key: true,
    disabled_at: null,
  },
];

function createApiSpies() {
  return {
    getRuntimeSettings: vi.spyOn(apiClient, "getRuntimeSettings").mockResolvedValue(runtimeResponse as never),
    updateRuntimeSettings: vi.spyOn(apiClient, "updateRuntimeSettings").mockResolvedValue(runtimeResponse as never),
    getDoclingSettings: vi.spyOn(apiClient, "getDoclingSettings").mockResolvedValue(doclingResponse as never),
    getSearchSettings: vi.spyOn(apiClient, "getSearchSettings").mockResolvedValue(searchSettingsResponse as never),
    updateDoclingSettings: vi.spyOn(apiClient, "updateDoclingSettings").mockResolvedValue(doclingResponse as never),
    updateSearchSettings: vi.spyOn(apiClient, "updateSearchSettings").mockResolvedValue({
      ...searchSettingsResponse,
      has_api_key: true,
    } as never),
    listProviderAccounts: vi.spyOn(apiClient, "listProviderAccounts").mockResolvedValue(providerAccountsResponse as never),
    createProviderAccount: vi.spyOn(apiClient, "createProviderAccount").mockResolvedValue(providerAccountsResponse[0] as never),
    updateProviderAccount: vi.spyOn(apiClient, "updateProviderAccount").mockResolvedValue(providerAccountsResponse[0] as never),
    deleteProviderAccount: vi.spyOn(apiClient, "deleteProviderAccount").mockResolvedValue(undefined as never),
    listAdminUsers: vi.spyOn(apiClient, "listAdminUsers").mockResolvedValue([] as never),
    createAdminUser: vi.spyOn(apiClient, "createAdminUser").mockResolvedValue(undefined as never),
    updateAdminUser: vi.spyOn(apiClient, "updateAdminUser").mockResolvedValue(undefined as never),
    resetAdminUserPassword: vi.spyOn(apiClient, "resetAdminUserPassword").mockResolvedValue(undefined as never),
    disableAdminUser: vi.spyOn(apiClient, "disableAdminUser").mockResolvedValue(undefined as never),
    enableAdminUser: vi.spyOn(apiClient, "enableAdminUser").mockResolvedValue(undefined as never),
  };
}

let apiSpies: ReturnType<typeof createApiSpies>;

describe("SettingsView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    installMockStorage();
    setGuest();
    apiSpies = createApiSpies();
  });

  it("loads settings, shows recent search history, and saves runtime/docling/search updates", async () => {
    window.localStorage.setItem(SEARCH_HISTORY_STORAGE_KEY, JSON.stringify([
      {
        query: "policy",
        sourceKey: "",
        publishedAfter: "",
        publishedBefore: "",
        limit: 8,
        savedAt: "2026-04-30T00:00:00.000Z",
      },
    ]));

    const wrapper = mount(SettingsView, {
      attachTo: document.body,
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n()],
      },
    });

    await flushPromises();
    await vi.waitFor(() => {
      expect(apiSpies.getRuntimeSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.getDoclingSettings).toHaveBeenCalledTimes(1);
      expect(apiSpies.getSearchSettings).toHaveBeenCalledTimes(1);
      expect(wrapper.find("#docling-base-url").exists()).toBe(true);
    });

    expect(wrapper.text()).toContain("Appearance");
    expect(wrapper.find('[data-testid="settings-locale-select"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="settings-theme-select"]').exists()).toBe(true);
    expect(wrapper.get("#runtime-scheduler-valkey-url").attributes("placeholder")).toBe("redis://valkey:6379/0");
    expect(wrapper.text()).toContain("Provider Accounts");
    expect(wrapper.text()).toContain("Recent Search History");

    await wrapper.get("#runtime-embedding-model").setValue("text-embedding-3-small");
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await wrapper.get("#search-rerank-api-key").setValue("rerank-secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateRuntimeSettings).toHaveBeenCalledWith(expect.objectContaining({
      embedding: expect.objectContaining({
        model: "text-embedding-3-small",
        provider_account_key: "openrouter-default",
      }),
      scheduler: expect.objectContaining({
        valkey_url: "redis://valkey:6379/0",
      }),
    }));
    expect(apiSpies.updateDoclingSettings).toHaveBeenCalledWith(expect.objectContaining({
      connection: expect.objectContaining({
        base_url: "http://docling.internal:5001",
      }),
      vlm: expect.objectContaining({
        provider_account_key: "openrouter-default",
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
    const wrapper = mount(SettingsView, {
      attachTo: document.body,
      global: {
        plugins: [testPrimeVuePlugin, i18n],
      },
    });

    await flushPromises();

    const selects = wrapper.findAllComponents(Select).filter((component) => {
      const testId = component.attributes()["data-testid"];
      return testId === "settings-locale-select" || testId === "settings-theme-select";
    });

    expect(selects).toHaveLength(2);

    await selects[0].vm.$emit("update:modelValue", "zh-CN");
    await selects[1].vm.$emit("update:modelValue", "light");

    expect(i18n.global.locale.value).toBe("zh-CN");
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe("zh-CN");
    expect(window.localStorage.getItem("context69.theme")).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("saves provider account changes through the single page save action", async () => {
    const wrapper = mount(SettingsView, {
      attachTo: document.body,
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n()],
      },
    });

    await flushPromises();
    await wrapper.get("#provider-clear-api-key").trigger("click");
    expect(wrapper.find("#provider-save-account").exists()).toBe(false);
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(apiSpies.updateProviderAccount).toHaveBeenCalledWith(expect.objectContaining({
      account_key: "openrouter-default",
      clear_api_key: true,
    }));
  });
});
