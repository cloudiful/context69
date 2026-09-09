import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  ApiError,
  apiClient,
  type GroupPageResponse,
  type GroupResponse,
  type SearchHit,
  type SearchResponse,
  type SourcePageResponse,
  type SourceStatus,
} from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { installMockStorage } from "../test-utils/storage";
import { SEARCH_HISTORY_STORAGE_KEY, type SearchHistoryEntry } from "../utils/search-history";
import { installFakeEventSource, takeEventSource } from "../test-utils/event-source";

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

function searchPage(
  query: string,
  hits: SearchHit[],
  pageSize = 8,
  overrides: Partial<SearchResponse["pagination"]> = {},
): SearchResponse {
  const total = hits.length;
  return {
    query,
    items: hits,
    pagination: {
      page: 1,
      page_size: pageSize,
      total,
      total_pages: total === 0 ? 0 : Math.ceil(total / pageSize),
      ...overrides,
    },
  };
}

function groupPage(items: GroupResponse[]): GroupPageResponse {
  return { items, pagination: { page: 1, page_size: 100, total: items.length, total_pages: items.length === 0 ? 0 : 1 } };
}

function hit(title: string, documentId: number, chunkId: string, extra: Partial<SearchHit> = {}): SearchHit {
  return {
    chunk_id: chunkId,
    document_id: documentId,
    source_key: "src",
    external_id: `ext-${chunkId}`,
    group_key: "g",
    group_path: "g/p",
    visibility: "private",
    title,
    summary: "",
    source_uri: "https://example.com/doc",
    published_at: null,
    chunk_index: 0,
    chunk_text: `${title} body text`,
    score: 0.5,
    metadata_json: {},
    ...extra,
  };
}

async function makeRouter(path = "/search") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/search", name: "search", component: SearchView },
      { path: "/documents/:id", name: "document", component: { template: "<div />" } },
      { path: "/groups/:groupPath", name: "group-overview", component: { template: "<div>library</div>" } },
    ],
  });
  router.push(path);
  await router.isReady();
  return router;
}

async function mountSearch(options: { path?: string; locale?: "en" | "zh-CN" } = {}) {
  const router = await makeRouter(options.path ?? "/search");
  const wrapper = mount(SearchView, {
    global: {
      plugins: [testNuxtUiPlugin, router, createTestI18n(options.locale ?? "en")],
    },
  });
  for (let index = 0; index < 6; index += 1) {
    await flushPromises();
  }
  return { router, wrapper };
}

async function settle() {
  for (let index = 0; index < 6; index += 1) {
    await flushPromises();
  }
}

describe("SearchView", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.addToast.mockReset();
    const storage = installMockStorage();
    storage.clear();
    window.sessionStorage.clear();
    window.sessionStorage.removeItem("context69.search-session");
    vi.spyOn(apiClient, "listGroups").mockResolvedValue(groupPage([]));
    vi.spyOn(apiClient, "listSources").mockResolvedValue(sourcePage([]));
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps the page empty before any search has been submitted", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("", []));
    const { wrapper } = await mountSearch();

    expect(search).not.toHaveBeenCalled();
    expect(wrapper.find(".search-results-panel").exists()).toBe(false);
  });

  it("loads route filters and renders search results", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("policy", [
        hit("Policy Circular", 7, "chunk-1", {
          source_key: "gov_documents",
          group_path: "personal-admin/default",
        }),
      ]),
    );
    const { wrapper, router } = await mountSearch({ path: "/search?q=policy&source=gov_documents" });

    expect(search).toHaveBeenCalledWith(
      expect.objectContaining({
        query: "policy",
        source_key: "gov_documents",
        page: 1,
      }),
      expect.any(Object),
    );
    expect(wrapper.find(".search-results-panel").exists()).toBe(true);
    expect(wrapper.findAll("h2").filter((node) => node.text() === "Search Results")).toHaveLength(1);
    expect(wrapper.text()).toContain("Policy Circular");
    expect(router.currentRoute.value.query.q).toBe("policy");
  });

  it("submits from the form, records history and reruns the selected entry", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("policy", []));
    const { wrapper } = await mountSearch();

    expect(wrapper.text()).not.toContain("Recent Searches");

    await wrapper.get("#query").setValue("policy");
    await wrapper.get("form").trigger("submit");
    await settle();

    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith(
      expect.objectContaining({ query: "policy", page: 1 }),
      expect.any(Object),
    );
    expect(window.localStorage.getItem(SEARCH_HISTORY_STORAGE_KEY)).toContain("policy");

    const autocomplete = wrapper.getComponent({ name: "InputMenu" });
    const suggestions = autocomplete.props("items") as SearchHistoryEntry[];
    expect(suggestions).toHaveLength(1);
    expect(suggestions[0]).toEqual(expect.objectContaining({ query: "policy" }));

    search.mockResolvedValueOnce(searchPage("policy", []));
    autocomplete.vm.$emit("update:modelValue", suggestions[0]);
    await settle();

    expect(search).toHaveBeenCalledTimes(2);
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "policy", page: 1 }),
      expect.any(Object),
    );
  });

  it("localizes the runtime-not-configured search error", async () => {
    vi.spyOn(apiClient, "search").mockRejectedValue(new Error(
      "search runtime is not configured; save runtime settings and restart the service",
    ));
    await mountSearch({ path: "/search?q=deepseek", locale: "zh-CN" });

    expect(mocks.addToast).toHaveBeenCalledWith({
      color: "error",
      title: "错误",
      description: "搜索运行时未配置。请先保存运行时设置，然后重启服务。",
      duration: 5000,
    });
  });

  it("renders cursor-navigated page 2 from a deep link and pages back with prev_cursor", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("beta", [hit("Beta Two", 2, "c2")], 8, {
        page: 2,
        total: 17,
        total_pages: 3,
        has_more: true,
        prev_cursor: "c1:prev8",
        next_cursor: "c1:next16",
      }),
    );
    const { wrapper, router } = await mountSearch({ path: "/search?q=beta&cursor=c1:abc8" });

    expect(search).toHaveBeenCalledWith(
      expect.objectContaining({ query: "beta", cursor: "c1:abc8" }),
      expect.any(Object),
    );
    expect(wrapper.text()).toContain("Beta Two");

    // Prev uses the page's own prev_cursor and syncs the URL cursor param.
    await wrapper.get('[data-testid="search-prev"]').trigger("click");
    await settle();
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "beta", cursor: "c1:prev8" }),
      expect.any(Object),
    );
    expect(router.currentRoute.value.query.cursor).toBe("c1:prev8");
    expect(router.currentRoute.value.query.page).toBeUndefined();
  });

  it("falls back to legacy page stepping when a page has no cursors", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("paged", [hit("Paged", 1, "c1")], 8, {
        page: 1,
        total: 9,
        total_pages: 2,
        has_more: true,
      }),
    );
    const { wrapper, router } = await mountSearch({ path: "/search?q=paged" });

    await wrapper.get('[data-testid="search-next"]').trigger("click");
    await settle();
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "paged", page: 2, cursor: undefined }),
      expect.any(Object),
    );
    expect(router.currentRoute.value.query.page).toBe("2");
    expect(router.currentRoute.value.query.cursor).toBeUndefined();
  });

  it("resets a cursor-ordering 400 to a fresh first page with a notice", async () => {
    const search = vi.spyOn(apiClient, "search")
      .mockRejectedValueOnce(new ApiError("cursor ordering mismatch for reranked results", 400))
      .mockResolvedValueOnce(searchPage("gamma", [hit("Gamma", 1, "c-g")]));
    const { router } = await mountSearch({ path: "/search?q=gamma&cursor=c1:abc" });

    expect(search).toHaveBeenCalledTimes(2);
    expect(search).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ query: "gamma", cursor: "c1:abc" }),
      expect.any(Object),
    );
    expect(search).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ query: "gamma", page: 1, cursor: undefined }),
      expect.any(Object),
    );
    expect(mocks.addToast).toHaveBeenCalledWith(
      expect.objectContaining({
        color: "error",
        description: "This page expired for the current result order; restarting from the first page.",
      }),
    );
    expect(router.currentRoute.value.query.cursor).toBeUndefined();
  });

  it("passes group_path through the payload and drops it together with cursor on group change", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("gamma", []));
    const { wrapper } = await mountSearch({ path: "/search?q=gamma&group_path=stock/sgs-disclosures" });

    const searchForm = wrapper.getComponent({ name: "SearchForm" });
    await searchForm.vm.$emit("group-change", "stock/other");
    await settle();

    expect(search).toHaveBeenCalledTimes(2);
    expect(search).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "gamma", group_path: "stock/other", page: 1, cursor: undefined }),
      expect.any(Object),
    );
  });

  it("opens the preview modal from a row preview action", async () => {
    vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("preview", [hit("Preview Doc", 3, "c-preview", { match_reason: "semantic+title" })]),
    );
    const { wrapper } = await mountSearch({ path: "/search?q=preview" });

    expect(wrapper.find('[data-testid="search-result-preview"]').exists()).toBe(true);
    await wrapper.find('[data-testid="search-result-preview"]').trigger("click");
    await settle();

    const model = wrapper.getComponent({ name: "Modal" }).props("open");
    expect(model).toBe(true);
    expect(wrapper.text()).toContain("Preview Doc");
  });

  it("labels lower-bound totals as at least instead of exact", async () => {
    vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("policy", [hit("Lower Bound", 11, "c-lower")], 8, {
        total: 9,
        total_pages: 2,
        has_more: true,
        total_is_exact: false,
      }),
    );
    const { wrapper } = await mountSearch({ path: "/search?q=policy" });

    expect(wrapper.text()).toContain("At least 9 results");
    expect(wrapper.text()).not.toContain("Results: 9");
  });

  it("renders Chinese lower-bound copy and refining/fallback strings for zh-CN", async () => {
    vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("policy", [], 8, { total: 9, total_pages: 2, has_more: true, total_is_exact: false }),
    );
    const { wrapper } = await mountSearch({ path: "/search?q=policy", locale: "zh-CN" });
    expect(wrapper.text()).toContain("至少 9 条");
  });

  it("keeps lightweight return context when opening hits", async () => {
    vi.spyOn(apiClient, "search").mockResolvedValue(
      searchPage("gamma", [hit("Gamma", 2, "c2", { match_reason: "semantic" })]),
    );
    const { wrapper, router } = await mountSearch({ path: "/search?q=gamma" });

    await wrapper.find('[data-testid="search-result-open"]').trigger("click");
    await settle();
    expect(router.currentRoute.value.name).toBe("document");
    const rawSession = window.sessionStorage.getItem("context69.search-session") ?? "";
    expect(rawSession).toContain("gamma");
    expect(rawSession).not.toContain("chunk_id");
  });

  it("reruns when the same-route query changes externally", async () => {
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("beta", []));
    const { router } = await mountSearch({ path: "/search?q=beta&page=1" });
    expect(search).toHaveBeenCalledTimes(1);

    search.mockResolvedValueOnce(searchPage("delta", []));
    await router.push("/search?q=delta&page=1");
    await settle();
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({ query: "delta", page: 1 }), expect.any(Object));
  });

  describe("streaming search", () => {
    it("renders the local page, then reorders in place on reranked and ends with done", async () => {
      installFakeEventSource();
      const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchPage("stream", []));
      const first = hit("Local First", 1, "c-first");
      const second = hit("Reranked Winner", 2, "c-second");
      const { wrapper } = await mountSearch({ path: "/search?q=stream" });

      const es = takeEventSource();
      expect(es.url).toContain("/v1/search/stream");
      expect(es.url).toContain("query=stream");

      es.emit("local", {
        items: [first, second],
        pagination: { page: 1, page_size: 8, total: 2, total_pages: 1, has_more: false },
      });
      await settle();
      expect(wrapper.find('[data-testid="search-refining"]').exists()).toBe(true);
      const titles = () =>
        wrapper
          .findAll('[data-testid="search-result-item"]')
          .map((node) => node.get('[data-testid="search-result-select"]').text());
      expect(titles()).toEqual(["Local First", "Reranked Winner"]);

      // The reranked frame reorders the already-rendered page in place.
      es.emit("reranked", {
        items: [second, first],
        pagination: { page: 1, page_size: 8, total: 2, total_pages: 1, has_more: false },
      });
      await settle();
      expect(titles()).toEqual(["Reranked Winner", "Local First"]);
      es.emit("done", { rerank_applied: true });
      await settle();

      expect(wrapper.find('[data-testid="search-refining"]').exists()).toBe(false);
      expect(wrapper.find('[data-testid="search-stream-fallback-notice"]').exists()).toBe(false);
      // The SSE exchange succeeded: POST was never used.
      expect(search).not.toHaveBeenCalled();
    });

    it("falls back to POST with a subtle notice when the stream fails", async () => {
      installFakeEventSource();
      const search = vi.spyOn(apiClient, "search").mockResolvedValue(
        searchPage("fallback", [hit("Fallback Result", 5, "c-fb")]),
      );
      const { wrapper } = await mountSearch({ path: "/search?q=fallback" });

      const es = takeEventSource();
      es.fail();
      await settle();

      expect(search).toHaveBeenCalledTimes(1);
      expect(search).toHaveBeenCalledWith(
        expect.objectContaining({ query: "fallback", page: 1 }),
        expect.any(Object),
      );
      expect(wrapper.text()).toContain("Fallback Result");
      expect(wrapper.find('[data-testid="search-stream-fallback-notice"]').exists()).toBe(true);
    });
  });
});
