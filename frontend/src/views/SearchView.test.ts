import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testPrimeVuePlugin } from "../test-utils/primevue";
import { installMockStorage } from "../test-utils/storage";
import { SEARCH_HISTORY_STORAGE_KEY } from "../utils/search-history";

import SearchView from "./SearchView.vue";

describe("SearchView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    installMockStorage();
  });

  it("keeps the page empty before any search has been submitted", async () => {
    const listSources = vi.spyOn(apiClient, "listSources").mockResolvedValue([]);
    const search = vi.spyOn(apiClient, "search").mockResolvedValue({ query: "", hits: [] });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });

    router.push("/search");
    await router.isReady();

    const wrapper = mount(SearchView, {
      global: {
        plugins: [testPrimeVuePlugin, router, createTestI18n()],
      },
    });

    await flushPromises();

    expect(search).not.toHaveBeenCalled();
    expect(wrapper.find(".search-results-panel").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Ready");
    expect(wrapper.text()).not.toContain("Set filters and run a search to view indexed content.");
  });

  it("loads route filters and renders search results", async () => {
    const listSources = vi.spyOn(apiClient, "listSources").mockResolvedValue([
      {
        source_key: "gov_documents",
        group_key: "personal-admin",
        group_path: "personal-admin/default",
        visibility: "private",
        display_name: "国务院/部委政策公文",
        description: "覆盖国务院及部委正式政策公文。",
        example_queries: ["新能源汽车 购置税 政策"],
        connection: "gov-info",
        has_database_url: true,
        origin_status: "connected",
        origin_message: null,
        sync_strategy: "cursor",
        connector_type: "postgres_sql",
        base_query: "SELECT 1",
        batch_size: 200,
        last_cursor_updated_at: null,
        last_cursor_external_id: null,
        last_success_at: null,
      },
    ]);
    const search = vi.spyOn(apiClient, "search").mockResolvedValue({
      query: "policy",
      hits: [
        {
          chunk_id: "chunk-1",
          document_id: 7,
          source_key: "gov_documents",
          external_id: "ext-7",
          group_key: "personal-admin",
          group_path: "personal-admin/default",
          visibility: "private",
          title: "Policy Circular",
          summary: "summary text",
          source_uri: "https://example.com/policy",
          published_at: "2025-01-02",
          chunk_index: 0,
          chunk_text: "important policy text",
          score: 0.87,
          metadata_json: {},
        },
      ],
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });

    router.push("/search?q=policy&source=gov_documents");
    await router.isReady();

    const wrapper = mount(SearchView, {
      global: {
        plugins: [testPrimeVuePlugin, router, createTestI18n()],
      },
    });

    await flushPromises();

    expect(search).toHaveBeenCalledWith(
      expect.objectContaining({
        query: "policy",
        source_key: "gov_documents",
      }),
      expect.any(Object),
    );
    expect(wrapper.find(".search-results-panel").exists()).toBe(true);
    expect(wrapper.findAll("h2").filter((node) => node.text() === "Search Results")).toHaveLength(1);
    expect(wrapper.findAll(".section-label").map((node) => node.text())).not.toContain("Results");
    expect(wrapper.text()).toContain("Policy Circular");
    expect(wrapper.text()).not.toContain("Query Console");
    expect(wrapper.text()).not.toContain("Matrix Retrieval Console");
  });

  it("stores recent searches, reruns them, and clears local history", async () => {
    const listSources = vi.spyOn(apiClient, "listSources").mockResolvedValue([]);
    const search = vi.spyOn(apiClient, "search").mockResolvedValue({
      query: "policy",
      hits: [],
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });

    router.push("/search");
    await router.isReady();

    const wrapper = mount(SearchView, {
      global: {
        plugins: [testPrimeVuePlugin, router, createTestI18n()],
      },
    });

    await flushPromises();
    expect(wrapper.text()).not.toContain("Recent Searches");

    await wrapper.get("#query").setValue("policy");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(search).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("Recent Searches");
    expect(window.localStorage.getItem(SEARCH_HISTORY_STORAGE_KEY)).toContain("policy");

    search.mockResolvedValueOnce({
      query: "policy",
      hits: [],
    });

    const historyButton = wrapper.findAll("button").find((button) => button.text().includes("policy"));
    expect(historyButton).toBeTruthy();
    await historyButton!.trigger("click");
    await flushPromises();

    expect(search).toHaveBeenCalledTimes(2);

    const clearButton = wrapper.findAll("button").find((button) => button.text() === "Clear Recent");
    expect(clearButton).toBeTruthy();
    await clearButton!.trigger("click");

    expect(window.localStorage.getItem(SEARCH_HISTORY_STORAGE_KEY)).toBeNull();
    expect(wrapper.text()).not.toContain("Recent Searches");
  });

  it("localizes the runtime-not-configured search error", async () => {
    const addToast = vi.fn();
    vi.spyOn(apiClient, "listSources").mockResolvedValue([]);
    vi.spyOn(apiClient, "search").mockRejectedValue(new Error(
      "search runtime is not configured; save runtime settings and restart the service",
    ));

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });

    router.push("/search?q=deepseek");
    await router.isReady();

    const wrapper = mount(SearchView, {
      global: {
        plugins: [
          testPrimeVuePlugin,
          {
            install(app) {
              app.config.globalProperties.$toast.add = addToast;
            },
          },
          router,
          createTestI18n("zh-CN"),
        ],
      },
    });

    await flushPromises();

    expect(addToast).toHaveBeenCalledWith({
      severity: "error",
      summary: "错误",
      detail: "搜索运行时未配置。请先保存运行时设置，然后重启服务。",
      life: 5000,
    });
    expect(wrapper.text()).not.toContain("搜索运行时未配置。请先保存运行时设置，然后重启服务。");
    expect(wrapper.text()).not.toContain("search runtime is not configured");
  });
});
