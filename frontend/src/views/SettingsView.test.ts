import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { installMockStorage } from "../test-utils/storage";
import { SEARCH_HISTORY_STORAGE_KEY } from "../utils/search-history";

const {
  getRuntimeSettings,
  updateRuntimeSettings,
  getDoclingSettings,
  getSearchSettings,
  updateDoclingSettings,
  updateSearchSettings,
  listProviderAccounts,
  createProviderAccount,
  updateProviderAccount,
} = vi.hoisted(() => ({
  getRuntimeSettings: vi.fn(),
  updateRuntimeSettings: vi.fn(),
  getDoclingSettings: vi.fn(),
  getSearchSettings: vi.fn(),
  updateDoclingSettings: vi.fn(),
  updateSearchSettings: vi.fn(),
  listProviderAccounts: vi.fn(),
  createProviderAccount: vi.fn(),
  updateProviderAccount: vi.fn(),
}));

vi.mock("../services/api", () => ({
  apiClient: {
    getRuntimeSettings,
    updateRuntimeSettings,
    getDoclingSettings,
    getSearchSettings,
    updateDoclingSettings,
    updateSearchSettings,
    listProviderAccounts,
    createProviderAccount,
    updateProviderAccount,
    deleteProviderAccount: vi.fn(),
  },
}));

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
  conversion: {},
  ocr: {
    do_ocr: true,
    force_ocr: false,
    ocr_lang: [],
  },
  enrichment: {},
  vlm: {
    provider_account_key: "openrouter-default",
    vlm_pipeline_model: "gemini-3-flash",
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

describe("SettingsView", () => {
  beforeEach(() => {
    installMockStorage();
    getRuntimeSettings.mockReset();
    updateRuntimeSettings.mockReset();
    getDoclingSettings.mockReset();
    getSearchSettings.mockReset();
    updateDoclingSettings.mockReset();
    updateSearchSettings.mockReset();
    listProviderAccounts.mockReset();
    createProviderAccount.mockReset();
    updateProviderAccount.mockReset();

    getRuntimeSettings.mockResolvedValue(runtimeResponse);
    updateRuntimeSettings.mockResolvedValue(runtimeResponse);
    getDoclingSettings.mockResolvedValue(doclingResponse);
    getSearchSettings.mockResolvedValue(searchSettingsResponse);
    updateDoclingSettings.mockResolvedValue(doclingResponse);
    updateSearchSettings.mockResolvedValue({
      ...searchSettingsResponse,
      has_api_key: true,
    });
    listProviderAccounts.mockResolvedValue(providerAccountsResponse);
    createProviderAccount.mockResolvedValue(providerAccountsResponse[0]);
    updateProviderAccount.mockResolvedValue(providerAccountsResponse[0]);
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
      expect(getRuntimeSettings).toHaveBeenCalledTimes(1);
      expect(getDoclingSettings).toHaveBeenCalledTimes(1);
      expect(getSearchSettings).toHaveBeenCalledTimes(1);
      expect(wrapper.find("#docling-base-url").exists()).toBe(true);
    });

    expect(wrapper.text()).toContain("Provider Accounts");
    expect(wrapper.text()).toContain("1 recent searches stored in this browser.");

    await wrapper.get("#runtime-embedding-model").setValue("text-embedding-3-small");
    await wrapper.get("#docling-base-url").setValue("http://docling.internal:5001");
    await wrapper.get("#search-rerank-api-key").setValue("rerank-secret");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(updateRuntimeSettings).toHaveBeenCalledWith(expect.objectContaining({
      embedding: expect.objectContaining({
        model: "text-embedding-3-small",
        provider_account_key: "openrouter-default",
      }),
      scheduler: expect.objectContaining({
        valkey_url: "redis://valkey:6379/0",
      }),
    }));
    expect(updateDoclingSettings).toHaveBeenCalledWith(expect.objectContaining({
      connection: expect.objectContaining({
        base_url: "http://docling.internal:5001",
      }),
      vlm: expect.objectContaining({
        provider_account_key: "openrouter-default",
      }),
    }));
    expect(updateSearchSettings).toHaveBeenCalledWith(expect.objectContaining({
      mode: "hybrid",
      rerank_enabled: true,
      api_key: "rerank-secret",
      clear_api_key: false,
    }));
    expect(wrapper.text()).toContain("Settings saved");
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

    expect(updateProviderAccount).toHaveBeenCalledWith(expect.objectContaining({
      account_key: "openrouter-default",
      clear_api_key: true,
    }));
  });

  it("scrolls to the matching section when clicking a left navigation item", async () => {
    const replaceState = vi.spyOn(window.history, "replaceState");
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });

    const wrapper = mount(SettingsView, {
      attachTo: document.body,
      global: {
        plugins: [testPrimeVuePlugin, createTestI18n()],
      },
    });

    await flushPromises();

    const schedulerLink = wrapper
      .findAll("button")
      .find((node) => node.text().trim() === "调度器" || node.text().trim() === "Scheduler");

    expect(schedulerLink).toBeTruthy();
    await schedulerLink!.trigger("click");

    expect(scrollIntoView).toHaveBeenCalledWith({
      block: "start",
      behavior: "smooth",
    });
    expect(replaceState).toHaveBeenCalledWith(null, "", "#settings-scheduler");
  });
});
