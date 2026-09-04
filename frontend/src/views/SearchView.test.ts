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
    pagination: { page, page_size: pageSize, total, total_pages: total === 0 ? 0 : Math.ceil(total / pageSize) },
  };
}

function searchPage(query: string, hits: SearchHit[], pageSize = 8): SearchResponse {
  const total = hits.length;
  return {
    query,
    items: hits,
    pagination: { page: 1, page_size: pageSize, total, total_pages: total === 0 ? 0 : Math.ceil(total / pageSize) },
  };
}

describe("SearchView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.addToast.mockReset();
    const storage = installMockStorage();
    storage.clear();
    window.sessionStorage.clear();
    window.sessionStorage.removeItem("context69.search-session");
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

  it("does not write [object Object] when history entry object is selected", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("alpha", []));

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
      global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] },
    });
    await flushPromises();

    await wrapper.get("#query").setValue("alpha");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    const autocomplete = wrapper.getComponent({ name: "InputMenu" });
    const suggestions = autocomplete.props("items") as SearchHistoryEntry[];
    // simulate history object emission (old bug would have set query to "[object Object]")
    autocomplete.vm.$emit("update:modelValue", suggestions[0] as unknown as string);
    await flushPromises();

    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "alpha" }),
      expect.any(Object),
    );
    expect(wrapper.get("#query").element).toBeDefined();
    expect(wrapper.text()).not.toContain("[object Object]");
    const emittedQuery = (search.mock.calls[search.mock.calls.length - 1]?.[0] as { query: string })?.query;
    expect(emittedQuery).toBe("alpha");
    expect(emittedQuery).not.toBe("[object Object]");
  });

  it("persists page and limit in URL and reruns when same route query changes", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("beta", [
      {
        chunk_id: "c1",
        document_id: 1,
        source_key: "src",
        external_id: "e1",
        group_key: "g",
        group_path: "g/p",
        visibility: "private",
        title: "Beta",
        summary: "",
        source_uri: "https://example.com/beta",
        published_at: null,
        chunk_index: 0,
        chunk_text: "t",
        score: 0.5,
        metadata_json: {},
      },
    ], 8));
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=beta&limit=16&page=2");
    await router.isReady();

    mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    expect(search).toHaveBeenCalledWith(
      expect.objectContaining({ query: "beta", limit: 16, page: 2 }),
      expect.any(Object),
    );
    expect(router.currentRoute.value.query.page).toBe("2");
    expect(router.currentRoute.value.query.limit).toBe("16");
  });

  it("does not overwrite a fresh empty search with a stored session", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("gamma", []));
    window.sessionStorage.setItem(
      "context69.search-session",
      JSON.stringify({ filters: { query: "gamma", sourceKey: "", publishedAfter: "", publishedBefore: "", limit: 8 }, page: 2 }),
    );

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div>doc</div>" } },
      ],
    });
    router.push("/search");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    // proactive empty entry stays empty instead of rerunning the stored session
    expect(search).not.toHaveBeenCalled();
    expect(wrapper.find(".search-results-panel").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("[object Object]");
  });

  it("keeps lightweight return context when opening hits without embedding the hit", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const gammaHit = {
      chunk_id: "c2",
      document_id: 2,
      source_key: "src",
      external_id: "e2",
      group_key: "g",
      group_path: "g/p",
      visibility: "private",
      title: "Gamma",
      summary: "",
      source_uri: "https://example.com/gamma",
      published_at: null,
      chunk_index: 0,
      chunk_text: "t2",
      score: 0.6,
      metadata_json: {},
    };
    vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("gamma", [gammaHit as SearchHit]));
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div>library</div>" } },
        { path: "/documents/:id", name: "document", component: { template: "<div>doc</div>" } },
      ],
    });
    router.push("/search?q=gamma");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    await wrapper.find('[data-testid="search-result-open"]').trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.name).toBe("document");
    expect(router.currentRoute.value.query.from).toBe("search");
    expect(JSON.stringify(router.currentRoute.value.query)).not.toContain("chunk_id");
    expect(JSON.stringify(router.currentRoute.value.fullPath)).not.toContain("[object Object]");
    const rawSession = window.sessionStorage.getItem("context69.search-session") ?? "";
    expect(rawSession).toContain("gamma");
    expect(rawSession).not.toContain("chunk_id");
  });

  it("reruns when the same-route query changes", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("beta", []));
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=beta&page=1");
    await router.isReady();
    mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();
    expect(search).toHaveBeenCalledTimes(1);

    search.mockResolvedValueOnce(searchPage("delta", []));
    await router.push("/search?q=delta&page=1");
    await flushPromises();
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ query: "delta", page: 1 }), expect.any(Object));
  });

  it("updates limit before requesting on page-size change and page in URL on page change", async () => {
    const hits = [
      {
        chunk_id: "c9",
        document_id: 9,
        source_key: "src",
        external_id: "e9",
        group_key: "g",
        group_path: "g/p",
        visibility: "private",
        title: "Paged",
        summary: "",
        source_uri: "https://example.com/paged",
        published_at: null,
        chunk_index: 0,
        chunk_text: "t",
        score: 0.5,
        metadata_json: {},
      },
    ];
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("paged", hits as SearchHit[]));
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=paged");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();
    expect(search).toHaveBeenCalledWith(expect.objectContaining({ query: "paged", limit: 8, page: 1 }), expect.any(Object));

    const resultList = wrapper.getComponent({ name: "SearchResultList" });
    resultList.vm.$emit("page-size", 16);
    await flushPromises();
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ query: "paged", limit: 16, page: 1 }), expect.any(Object));
    expect(router.currentRoute.value.query.limit).toBe("16");

    const freshList = wrapper.getComponent({ name: "SearchResultList" });
    await freshList.vm.$emit("page", 2);
    await flushPromises();
    await flushPromises();
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ query: "paged", limit: 16, page: 2 }), expect.any(Object));
    expect(router.currentRoute.value.query.page).toBe("2");
  });

  it("labels lower-bound totals as at least instead of exact", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    vi.spyOn(apiClient, "search").mockResolvedValue({
      query: "policy",
      items: [
        {
          chunk_id: "c-lower",
          document_id: 11,
          source_key: "src",
          external_id: "e11",
          group_key: "g",
          group_path: "g/p",
          visibility: "private",
          title: "Lower Bound",
          summary: "",
          source_uri: "https://example.com/lower",
          published_at: null,
          chunk_index: 0,
          chunk_text: "lower bound text",
          score: 0.6,
          metadata_json: {},
        } as SearchHit,
      ],
      pagination: {
        page: 1,
        page_size: 8,
        total: 9,
        total_pages: 2,
        has_more: true,
        total_is_exact: false,
      },
    });
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=policy");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    expect(wrapper.text()).toContain("At least 9 results");
    expect(wrapper.text()).not.toContain("Results: 9");
  });

  it("keeps exact labels for legacy responses without window signals", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("legacy", [
      {
        chunk_id: "c-legacy",
        document_id: 12,
        source_key: "src",
        external_id: "e12",
        group_key: "g",
        group_path: "g/p",
        visibility: "private",
        title: "Legacy",
        summary: "",
        source_uri: "https://example.com/legacy",
        published_at: null,
        chunk_index: 0,
        chunk_text: "legacy text",
        score: 0.5,
        metadata_json: {},
      } as SearchHit,
    ]));
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=legacy");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    expect(wrapper.text()).toContain("Results");
    expect(wrapper.text()).not.toContain("At least");
  });

  it("renders Chinese lower-bound copy for inexact windows", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    vi.spyOn(apiClient, "search").mockResolvedValue({
      query: "policy",
      items: [],
      pagination: {
        page: 1,
        page_size: 8,
        total: 9,
        total_pages: 2,
        has_more: true,
        total_is_exact: false,
      },
    });
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=policy");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n("zh-CN")] } });
    await flushPromises();

    expect(wrapper.text()).toContain("至少 9 条");
  });

  it("labels capped windows with unknown has_more as at least", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    vi.spyOn(apiClient, "search").mockResolvedValue({
      query: "policy",
      items: [
        {
          chunk_id: "c-capped",
          document_id: 14,
          source_key: "src",
          external_id: "e14",
          group_key: "g",
          group_path: "g/p",
          visibility: "private",
          title: "Capped",
          summary: "",
          source_uri: "https://example.com/capped",
          published_at: null,
          chunk_index: 0,
          chunk_text: "capped text",
          score: 0.5,
          metadata_json: {},
        } as SearchHit,
      ],
      pagination: {
        page: 1,
        page_size: 8,
        total: 5,
        total_pages: 1,
        total_is_exact: false,
      },
    });
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=policy");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    expect(wrapper.text()).toContain("At least 5 results");
    expect(wrapper.text()).not.toContain("Results: 5");
  });

  it("keeps paging on backend totals when the window is inexact", async () => {
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
    const search = vi.spyOn(apiClient, "search").mockResolvedValue({
      query: "paged",
      items: [
        {
          chunk_id: "c-paged",
          document_id: 13,
          source_key: "src",
          external_id: "e13",
          group_key: "g",
          group_path: "g/p",
          visibility: "private",
          title: "Paged Inexact",
          summary: "",
          source_uri: "https://example.com/paged-inexact",
          published_at: null,
          chunk_index: 0,
          chunk_text: "paged text",
          score: 0.5,
          metadata_json: {},
        } as SearchHit,
      ],
      pagination: {
        page: 1,
        page_size: 8,
        total: 9,
        total_pages: 2,
        has_more: true,
        total_is_exact: false,
      },
    });
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/search", name: "search", component: SearchView },
        { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      ],
    });
    router.push("/search?q=paged");
    await router.isReady();
    const wrapper = mount(SearchView, { global: { plugins: [testNuxtUiPlugin, router, createTestI18n()] } });
    await flushPromises();

    const resultList = wrapper.getComponent({ name: "SearchResultList" });
    resultList.vm.$emit("page", 2);
    await flushPromises();
    await flushPromises();
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ query: "paged", page: 2 }), expect.any(Object));
    expect(router.currentRoute.value.query.page).toBe("2");
  });
});
