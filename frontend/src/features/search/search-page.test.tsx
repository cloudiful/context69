import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { installMockStorage } from "@/test-utils/storage";
import { applyTimeZoneSelection, setLocale } from "@/lib/preferences-store";

import {
  FakeEventSource,
  installDomPolyfills,
  installFakeEventSource,
  installSearchBackend,
  openSearch,
  resetAuth,
  resetPreferences,
  resetToasts,
  searchHit,
  searchResponse,
} from "./test-support";

function streamPage(
  query: string,
  titles: string[],
  pagination: Parameters<typeof searchResponse>[2] = {},
) {
  return searchResponse(
    query,
    titles.map((title, index) => searchHit({
      chunk_id: `chunk-${title}`,
      title,
      score: 1 - index / 10,
      group_path: "personal-admin/default",
      published_at: "2026-10-01T08:00:00Z",
    })),
    pagination,
  );
}

async function firstStream(): Promise<FakeEventSource> {
  await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
  return FakeEventSource.instances[0];
}

function resultTitles(): string[] {
  return screen.queryAllByTestId("search-result-item").map((item) => (
    within(item).getByTestId("search-result-select").textContent ?? ""
  ));
}

async function expectResults(titles: string[]) {
  await waitFor(() => expect(resultTitles()).toEqual(titles));
}

describe("react search page", () => {
  beforeAll(() => installDomPolyfills());

  beforeEach(() => {
    resetToasts();
    installMockStorage();
    resetPreferences();
    applyTimeZoneSelection("UTC");
    resetAuth();
    installFakeEventSource();
  });

  afterEach(() => {
    resetToasts();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    FakeEventSource.instances = [];
  });

  it("stays empty before any search is submitted", async () => {
    const backend = installSearchBackend({ post: streamPage("policy", []) });
    await openSearch("/search");

    expect(screen.getByTestId("search-query")).toHaveValue("");
    expect(screen.queryByTestId("search-results-list")).not.toBeInTheDocument();
    expect(FakeEventSource.instances).toHaveLength(0);
    expect(backend.callsFor("POST", "/v1/search")).toHaveLength(0);
    expect(screen.queryByText("Recent searches")).not.toBeInTheDocument();
  });

  it("loads URL filters, streams local then reranked, and ends with done", async () => {
    const backend = installSearchBackend();
    const router = await openSearch(
      "/search?q=policy&source=gov_documents&limit=16&after=2026-01-01&before=2026-02-01&sort=date",
    );
    const source = await firstStream();

    const url = new URL(source.url);
    expect(url.pathname).toBe("/v1/search/stream");
    expect(url.searchParams.get("query")).toBe("policy");
    expect(url.searchParams.get("limit")).toBe("16");
    expect(url.searchParams.get("source_key")).toBe("gov_documents");
    expect(url.searchParams.get("published_after")).toBe("2026-01-01");
    expect(url.searchParams.get("published_before")).toBe("2026-02-01");
    expect(url.searchParams.get("sort")).toBe("date");
    expect(url.searchParams.get("page")).toBeNull();
    expect(source.withCredentials).toBe(true);

    act(() => {
      source.emit("local", streamPage("policy", ["Local First", "Reranked Winner"]));
    });

    const titles = () => screen
      .getAllByTestId("search-result-item")
      .map((item) => within(item).getByTestId("search-result-select").textContent);
    await waitFor(() => expect(titles()).toEqual(["Local First", "Reranked Winner"]));
    expect(screen.getByTestId("search-refining")).toBeInTheDocument();
    expect(screen.getByTestId("search-sort-mode")).toBeInTheDocument();

    act(() => {
      source.emit("reranked", streamPage("policy", ["Reranked Winner", "Local First"]));
    });
    await waitFor(() => expect(titles()).toEqual(["Reranked Winner", "Local First"]));

    act(() => {
      source.emit("done", { rerank_applied: true });
    });
    await waitFor(() => {
      expect(screen.queryByTestId("search-refining")).not.toBeInTheDocument();
    });

    expect(screen.queryByTestId("search-stream-fallback-notice")).not.toBeInTheDocument();
    expect(backend.callsFor("POST", "/v1/search")).toHaveLength(0);
    expect(router.state.location.search).toContain("q=policy");
    expect(screen.getByTestId("search-prev")).toBeDisabled();
  });

  it("falls back to POST with a subtle notice when the stream fails", async () => {
    const backend = installSearchBackend({ post: streamPage("fallback", ["Fallback Result"]) });
    await openSearch("/search?q=fallback");
    const source = await firstStream();

    act(() => {
      source.failNetwork();
    });

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/search")[0].body).toMatchObject({
      query: "fallback",
      page: 1,
      limit: 8,
    });
    await expectResults(["Fallback Result"]);
    expect(screen.getByTestId("search-stream-fallback-notice")).toBeInTheDocument();
  });

  it("falls back to POST on an in-band error frame", async () => {
    const backend = installSearchBackend({ post: streamPage("boom", ["Recovered Result"]) });
    await openSearch("/search?q=boom");
    const source = await firstStream();

    act(() => {
      source.failFrame();
    });

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    await expectResults(["Recovered Result"]);
  });

  it("pages deep-linked cursors through POST and keeps the URL in sync", async () => {
    const backend = installSearchBackend({
      post: [
        searchResponse("beta", [searchHit({ chunk_id: "c2", title: "Beta Two", score: 0.9 })], {
          page: 2,
          total: 17,
          total_pages: 3,
          has_more: true,
          prev_cursor: "c1:prev8",
          next_cursor: "c1:next16",
        }),
        searchResponse("beta", [searchHit({ chunk_id: "c3", title: "Beta Three", score: 0.8 })], {
          page: 3,
          total: 17,
          total_pages: 3,
          has_more: false,
          prev_cursor: "c1:prev16",
        }),
        searchResponse("beta", [searchHit({ chunk_id: "c1", title: "Beta One", score: 0.7 })], {
          page: 1,
          total: 17,
          total_pages: 3,
          has_more: true,
          next_cursor: "c1:next8",
        }),
      ],
    });
    const router = await openSearch("/search?q=beta&cursor=c1:abc8");

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/search")[0].body).toMatchObject({
      query: "beta",
      cursor: "c1:abc8",
      page: 1,
    });
    await expectResults(["Beta Two"]);
    expect(FakeEventSource.instances).toHaveLength(0);

    await userEvent.click(screen.getByTestId("search-next"));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(2);
    });
    expect(backend.callsFor("POST", "/v1/search")[1].body).toMatchObject({
      cursor: "c1:next16",
      page: 1,
    });
    await waitFor(() => {
      expect(new URLSearchParams(router.state.location.search).get("cursor")).toBe("c1:next16");
    });
    expect(new URLSearchParams(router.state.location.search).get("page")).toBeNull();

    await userEvent.click(screen.getByTestId("search-prev"));
    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(3);
    });
    expect(backend.callsFor("POST", "/v1/search")[2].body).toMatchObject({
      cursor: "c1:prev16",
    });
  });

  it("steps legacy pages through POST without cursors", async () => {
    const backend = installSearchBackend({
      post: [
        searchResponse("paged", [searchHit({ chunk_id: "p2", title: "Page Two", score: 0.5 })], {
          page: 2,
          total: 9,
          total_pages: 2,
          has_more: false,
        }),
      ],
    });
    const router = await openSearch("/search?q=paged");
    const source = await firstStream();

    act(() => {
      source.emit("local", streamPage("paged", ["Page One"], {
        page: 1,
        total: 9,
        total_pages: 2,
        has_more: true,
      }));
    });
    await expectResults(["Page One"]);

    await userEvent.click(screen.getByTestId("search-next"));

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/search")[0].body).toMatchObject({
      query: "paged",
      page: 2,
      cursor: undefined,
    });
    await expectResults(["Page Two"]);
    expect(FakeEventSource.instances).toHaveLength(1);
    await waitFor(() => {
      expect(new URLSearchParams(router.state.location.search).get("page")).toBe("2");
    });
    expect(new URLSearchParams(router.state.location.search).get("cursor")).toBeNull();
  });

  it("keeps date results forward-only", async () => {
    installSearchBackend();
    const router = await openSearch("/search?q=paged&sort=date");
    const source = await firstStream();

    act(() => {
      source.emit("local", streamPage("paged", ["Date One"], {
        page: 2,
        total: 9,
        total_pages: 2,
        has_more: false,
        prev_cursor: "c1:prev8",
      }));
    });
    await expectResults(["Date One"]);
    expect(screen.getByTestId("search-prev")).toBeDisabled();
    expect(screen.getByTestId("search-next")).toBeDisabled();
    expect(new URLSearchParams(router.state.location.search).get("page")).toBeNull();
    expect(new URLSearchParams(router.state.location.search).get("sort")).toBe("date");
  });

  it("resets a cursor-ordering rejection to a fresh first page with a notice", async () => {
    const backend = installSearchBackend({
      post: [{ status: 400, message: "cursor ordering mismatch for reranked results" }],
    });
    const router = await openSearch("/search?q=gamma&cursor=c1:abc");

    expect(await screen.findByText(
      "This page expired for the current result order; restarting from the first page.",
    )).toBeInTheDocument();

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(backend.callsFor("POST", "/v1/search")[0].body).toMatchObject({ cursor: "c1:abc" });

    const source = await firstStream();
    expect(new URL(source.url).searchParams.get("page")).toBeNull();
    expect(new URL(source.url).searchParams.get("cursor")).toBeNull();

    act(() => {
      source.emit("local", streamPage("gamma", ["Gamma"]));
    });
    await expectResults(["Gamma"]);
    await waitFor(() => {
      expect(new URLSearchParams(router.state.location.search).get("cursor")).toBeNull();
    });
  });

  it("drops the cursor when the folder scope changes", async () => {
    installSearchBackend({
      post: searchResponse("gamma", [searchHit({ chunk_id: "c-g", title: "Gamma", score: 1 })]),
      groups: [
        {
          group_id: 1,
          group_key: "stock/a",
          group_path: "stock/a",
          kind: "shared",
          name: "A",
          visibility: "public",
          created_at: "2026-01-01T08:00:00Z",
          updated_at: "2026-01-01T08:00:00Z",
        },
        {
          group_id: 2,
          group_key: "stock/b",
          group_path: "stock/b",
          kind: "shared",
          name: "B",
          visibility: "public",
          created_at: "2026-01-01T08:00:00Z",
          updated_at: "2026-01-01T08:00:00Z",
        },
      ],
    });
    const router = await openSearch("/search?q=gamma&group_path=stock/a&cursor=c1:abc");

    await expectResults(["Gamma"]);
    expect(FakeEventSource.instances).toHaveLength(0);

    await userEvent.click(screen.getByTestId("search-toggle-advanced"));
    await userEvent.click(await screen.findByTestId("search-group-path"));
    await userEvent.click(await screen.findByRole("option", { name: "stock / b" }));

    await waitFor(() => {
      expect(FakeEventSource.instances.length).toBe(1);
    });
    const second = FakeEventSource.instances[0];
    const url = new URL(second.url);
    expect(url.searchParams.get("group_path")).toBe("stock/b");
    expect(url.searchParams.get("cursor")).toBeNull();
    expect(url.searchParams.get("page")).toBeNull();
    await waitFor(() => {
      expect(new URLSearchParams(router.state.location.search).get("group_path")).toBe("stock/b");
    });
    expect(new URLSearchParams(router.state.location.search).get("cursor")).toBeNull();
  });

  it("records search history and replays an entry as a cursor-free first page", async () => {
    installSearchBackend();
    await openSearch("/search");

    await userEvent.type(screen.getByTestId("search-query"), "policy");
    await userEvent.click(screen.getByTestId("search-submit"));

    const first = await firstStream();
    expect(first.url).toContain("query=policy");

    act(() => {
      first.emit("local", streamPage("policy", ["Policy Result"]));
      first.emit("done", {});
    });

    await expectResults(["Policy Result"]);
    const stored = JSON.parse(window.localStorage.getItem("context69.search-history") ?? "[]") as Array<Record<string, unknown>>;
    expect(stored).toHaveLength(1);
    expect(stored[0].query).toBe("policy");
    expect(stored[0]).not.toHaveProperty("cursor");
    expect(stored[0]).not.toHaveProperty("page");

    await userEvent.click(screen.getByRole("button", { name: "Recent searches" }));
    const entry = await screen.findByTestId("search-history-entry");
    expect(entry).toHaveTextContent("policy");
    await userEvent.click(entry);

    await waitFor(() => {
      expect(FakeEventSource.instances.length).toBe(2);
    });
    expect(FakeEventSource.instances[1].url).toContain("query=policy");
    expect(new URLSearchParams(FakeEventSource.instances[1].url.split("?")[1] ?? "").get("cursor")).toBeNull();
  });

  it("keeps search and in-memory history usable when storage is denied", async () => {
    const original = Object.getOwnPropertyDescriptor(window, "localStorage");
    installSearchBackend();

    try {
      Object.defineProperty(window, "localStorage", {
        configurable: true,
        get: () => {
          throw new DOMException("Storage denied", "SecurityError");
        },
      });

      await openSearch("/search");
      await userEvent.type(screen.getByTestId("search-query"), "policy");
      await userEvent.click(screen.getByTestId("search-submit"));

      const source = await firstStream();
      act(() => {
        source.emit("local", streamPage("policy", ["Offline History Result"]));
      });
      await expectResults(["Offline History Result"]);

      await userEvent.click(screen.getByRole("button", { name: "Recent searches" }));
      expect(await screen.findByTestId("search-history-entry")).toHaveTextContent("policy");
    } finally {
      if (original) {
        Object.defineProperty(window, "localStorage", original);
      }
    }
  });

  it("reruns when the same-route query changes externally", async () => {
    installSearchBackend();
    const router = await openSearch("/search?q=beta");
    const first = await firstStream();

    act(() => {
      first.emit("local", streamPage("beta", ["Beta Result"]));
      first.emit("done", {});
    });
    await expectResults(["Beta Result"]);

    await act(async () => {
      await router.navigate("/search?q=delta");
    });

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2));
    const second = FakeEventSource.instances[1];
    expect(second.url).toContain("query=delta");

    act(() => {
      second.emit("local", streamPage("delta", ["Delta Result"]));
      second.emit("done", {});
    });
    await expectResults(["Delta Result"]);
    expect(new URLSearchParams(router.state.location.search).get("q")).toBe("delta");
  });

  it("restarts at page one when the page size changes", async () => {
    installSearchBackend();
    const router = await openSearch("/search?q=policy");
    const first = await firstStream();

    act(() => {
      first.emit("local", streamPage("policy", ["First Page"], {
        page: 1,
        page_size: 8,
        total: 20,
        total_pages: 3,
        has_more: true,
      }));
      first.emit("done", {});
    });
    await expectResults(["First Page"]);

    await userEvent.click(screen.getByTestId("search-page-size"));
    await userEvent.click(await screen.findByRole("option", { name: "16" }));

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2));
    const second = new URL(FakeEventSource.instances[1].url);
    expect(second.searchParams.get("query")).toBe("policy");
    expect(second.searchParams.get("limit")).toBe("16");
    expect(second.searchParams.get("page")).toBeNull();
    await waitFor(() => {
      expect(new URLSearchParams(router.state.location.search).get("limit")).toBe("16");
    });
    expect(new URLSearchParams(router.state.location.search).get("page")).toBeNull();
  });

  it("localizes the runtime-not-configured search error", async () => {
    installSearchBackend({
      postError: {
        status: 500,
        message: "search runtime is not configured; save runtime settings and restart the service",
      },
    });
    await openSearch("/search?q=deepseek");
    const source = await firstStream();

    act(() => {
      source.failNetwork();
    });

    expect(await screen.findByText(
      "Search runtime is not configured. Save runtime settings, then restart the service.",
    )).toBeInTheDocument();
  });

  it("renders the Chinese lower-bound, refining and fallback copy", async () => {
    installSearchBackend({ post: streamPage("policy", ["回退结果"]) });
    setLocale("zh-CN");
    await openSearch("/search?q=policy");
    const source = await firstStream();

    act(() => {
      source.emit("local", streamPage("policy", ["本地结果"], {
        total: 9,
        total_pages: 2,
        has_more: true,
        total_is_exact: false,
      }));
    });

    await waitFor(() => expect(screen.queryAllByTestId("search-result-item").length).toBeGreaterThan(0));
    expect(screen.getByText("至少 9 条")).toBeInTheDocument();
    expect(screen.getByTestId("search-refining")).toHaveTextContent("精排中");

    act(() => {
      source.failNetwork();
    });

    await waitFor(() => expect(screen.queryAllByTestId("search-result-item").length).toBeGreaterThan(0));
    expect(screen.getByTestId("search-stream-fallback-notice")).toHaveTextContent(
      "实时结果不可用，已显示标准结果。",
    );
    expect(screen.getByText("回退结果")).toBeInTheDocument();
  });

  it("shows a POST failure after the stream fails", async () => {
    installSearchBackend({ postError: { status: 500, message: "search rejected" } });
    await openSearch("/search?q=policy");
    const source = await firstStream();

    act(() => {
      source.failNetwork();
    });

    expect(await screen.findByText("search rejected")).toBeInTheDocument();
    expect(screen.queryByTestId("search-results-list")).not.toBeInTheDocument();
    expect(screen.queryByTestId("search-stream-fallback-notice")).not.toBeInTheDocument();
  });

  it("keeps search usable when sources or groups cannot be loaded", async () => {
    installSearchBackend({
      sourcesError: { status: 503, message: "sources down" },
      groupsError: { status: 503, message: "groups down" },
    });
    await openSearch("/search");

    expect(await screen.findByText("Failed to load sources")).toBeInTheDocument();
    expect(await screen.findByText("Failed to load groups")).toBeInTheDocument();

    await userEvent.type(screen.getByTestId("search-query"), "policy");
    await userEvent.click(screen.getByTestId("search-submit"));

    const source = await firstStream();
    act(() => {
      source.emit("local", streamPage("policy", ["Still Searchable"]));
    });
    await expectResults(["Still Searchable"]);
  });
});
