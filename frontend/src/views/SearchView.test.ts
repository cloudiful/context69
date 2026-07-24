import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AutoComplete from "@nuxt/ui/components/InputMenu.vue";

import { apiClient, type SearchHit, type SearchResponse, type SourcePageResponse, type SourceStatus } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { installMockStorage } from "../test-utils/storage";
import { SEARCH_HISTORY_STORAGE_KEY, type SearchHistoryEntry } from "../utils/search-history";

import SearchView from "./SearchView.vue";

const mocks = vi.hoisted(() => ({ addToast: vi.fn() }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.addToast }) }));

function sourcePage(items: SourceStatus[], page = 1, pageSize = 50): SourcePageResponse {
  const total = items.length;
  return {
    items,
    page,
    page_size: pageSize,
    total,
    total_pages: total === 0 ? 0 : Math.ceil(total / pageSize),
  };
}

function searchPage(query: string, hits: SearchHit[], pageSize = 8): SearchResponse {
  const total = hits.length;
  return {
    query,
    hits,
    page: 1,
    page_size: pageSize,
    total,
    total_pages: total === 0 ? 0 : Math.ceil(total / pageSize),
  };
}

describe("SearchView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.addToast.mockReset();
    installMockStorage();
  });

  it("keeps the page empty before any search has been submitted", async () => {
    const listSources = vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("", []));

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
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
      },
    });

    await flushPromises();

    expect(search).not.toHaveBeenCalled();
    expect(wrapper.find(".search-results-panel").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Ready");
    expect(wrapper.text()).not.toContain("Set filters and run a search to view indexed content.");
  });

  it("loads route filters and renders search results", async () => {
    const listSources = vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([
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
    ]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue({
      ...searchPage("policy", [
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
      ]),
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
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
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

  it("suggests recent searches from the query input and reruns the selected entry", async () => {
    const listSources = vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("policy", []));

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
        plugins: [testNuxtUiPlugin, router, createTestI18n()],
      },
    });

    await flushPromises();
    expect(wrapper.text()).not.toContain("Recent Searches");

    await wrapper.get("#query").setValue("policy");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(search).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).not.toContain("Recent Searches");
    expect(window.localStorage.getItem(SEARCH_HISTORY_STORAGE_KEY)).toContain("policy");

    search.mockResolvedValueOnce(searchPage("policy", []));

    const autocomplete = wrapper.getComponent({ name: "InputMenu" });
    const suggestions = autocomplete.props("items") as SearchHistoryEntry[];
    expect(suggestions).toHaveLength(1);
    expect(suggestions[0]).toEqual(expect.objectContaining({ query: "policy" }));

    autocomplete.vm.$emit("update:modelValue", suggestions[0]);
    await flushPromises();

    expect(search).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).not.toContain("Recent Searches");
  });

  it("localizes the runtime-not-configured search error", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
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
          testNuxtUiPlugin,
          router,
          createTestI18n("zh-CN"),
        ],
      },
    });

    await flushPromises();

    expect(mocks.addToast).toHaveBeenCalledWith({
      color: "error",
      title: "错误",
      description: "搜索运行时未配置。请先保存运行时设置，然后重启服务。",
      duration: 5000,
    });
    expect(wrapper.text()).not.toContain("搜索运行时未配置。请先保存运行时设置，然后重启服务。");
    expect(wrapper.text()).not.toContain("search runtime is not configured");
  });
});
