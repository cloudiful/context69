import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SearchRequest } from "@/services/api";

import {
  FakeEventSource,
  installFakeEventSource,
  installSearchBackend,
  searchHit,
  searchResponse,
} from "./test-support";
import {
  buildSearchStreamUrl,
  parseStreamPage,
  useSearchStream,
  type SearchStreamController,
} from "./use-search-stream";

const PAYLOAD: SearchRequest = { query: "policy", limit: 8, page: 1, sort: "relevance" };

function page(query: string, title: string, overrides: { nextCursor?: string } = {}) {
  return searchResponse(query, [searchHit({ chunk_id: `chunk-${title}`, title, score: 1 })], {
    next_cursor: overrides.nextCursor ?? null,
  });
}

function renderHarness(noticeMs?: number) {
  let controller: SearchStreamController | null = null;
  const onPage = vi.fn();

  function Harness() {
    controller = useSearchStream(onPage, { noticeMs });
    return (
      <div>
        <span data-testid="loading">{controller.loading ? "loading" : "idle"}</span>
        <span data-testid="refining">{controller.refining ? "refining" : "idle"}</span>
        <span data-testid="notice">{controller.fallbackNotice ? "notice" : "none"}</span>
      </div>
    );
  }

  const view = render(<Harness />);

  return {
    unmount: view.unmount,
    onPage,
    get controller() {
      if (!controller) {
        throw new Error("controller is not ready");
      }
      return controller;
    },
  };
}

describe("useSearchStream", () => {
  beforeEach(() => {
    installFakeEventSource();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    FakeEventSource.instances = [];
  });

  it("builds a typed first-page stream URL", () => {
    const url = new URL(buildSearchStreamUrl({
      query: "policy",
      limit: 16,
      page: 1,
      source_key: "gov",
      group_path: "stock/a",
      published_after: "2026-01-01",
      published_before: "2026-02-01",
      sort: "date",
    }));

    expect(url.pathname).toBe("/v1/search/stream");
    expect(url.searchParams.get("query")).toBe("policy");
    expect(url.searchParams.get("limit")).toBe("16");
    expect(url.searchParams.get("source_key")).toBe("gov");
    expect(url.searchParams.get("group_path")).toBe("stock/a");
    expect(url.searchParams.get("published_after")).toBe("2026-01-01");
    expect(url.searchParams.get("published_before")).toBe("2026-02-01");
    expect(url.searchParams.get("sort")).toBe("date");
  });

  it("rejects malformed frame payloads", () => {
    expect(parseStreamPage("not-json")).toBeNull();
    expect(parseStreamPage(JSON.stringify({ items: "no", pagination: {} }))).toBeNull();
    expect(parseStreamPage(JSON.stringify({
      items: [{ chunk_id: "c" }],
      pagination: { page: 1, page_size: 8, total: 1, total_pages: 1 },
    }))).toBeNull();
    expect(parseStreamPage(JSON.stringify(page("policy", "Policy")))).not.toBeNull();
  });

  it("uses POST directly for cursor-bearing requests and never opens a stream", async () => {
    const backend = installSearchBackend({ post: page("policy", "Cursor Page") });
    const harness = renderHarness();

    await act(async () => {
      await harness.controller.run({ ...PAYLOAD, cursor: "c1:next", page: 1 });
    });

    expect(FakeEventSource.instances).toHaveLength(0);
    expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    expect(backend.callsFor("POST", "/v1/search")[0].body).toMatchObject({ cursor: "c1:next", page: 1 });
    expect(harness.onPage).toHaveBeenCalledWith(expect.objectContaining({
      items: [expect.objectContaining({ title: "Cursor Page" })],
    }));
    expect(screen.getByTestId("loading")).toHaveTextContent("idle");
  });

  it("closes the previous stream and ignores its stale frames", async () => {
    installSearchBackend();
    const harness = renderHarness();

    act(() => {
      void harness.controller.run({ ...PAYLOAD, query: "first" });
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const first = FakeEventSource.instances[0];
    expect(first.withCredentials).toBe(true);

    act(() => {
      void harness.controller.run({ ...PAYLOAD, query: "second" });
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2));
    const second = FakeEventSource.instances[1];

    expect(first.closed).toBe(true);
    expect(first.listenerCount("local")).toBe(0);
    expect(second.closed).toBe(false);

    act(() => {
      first.emit("local", page("first", "Stale Result"));
    });
    expect(harness.onPage).not.toHaveBeenCalledWith(expect.objectContaining({
      items: [expect.objectContaining({ title: "Stale Result" })],
    }));

    act(() => {
      second.emit("local", page("second", "Current Result"));
    });
    expect(harness.onPage).toHaveBeenCalledWith(expect.objectContaining({
      items: [expect.objectContaining({ title: "Current Result" })],
    }));
  });

  it("commits local then reranked frames in place and ends on done", async () => {
    installSearchBackend();
    const harness = renderHarness();

    act(() => {
      void harness.controller.run(PAYLOAD);
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const source = FakeEventSource.instances[0];

    expect(source.url).toContain("/v1/search/stream");
    expect(source.url).toContain("query=policy");

    act(() => {
      source.emit("local", page("policy", "Local First"));
    });
    expect(screen.getByTestId("refining")).toHaveTextContent("refining");

    act(() => {
      source.emit("reranked", page("policy", "Reranked Winner"));
    });
    expect(harness.onPage).toHaveBeenLastCalledWith(expect.objectContaining({
      items: [expect.objectContaining({ title: "Reranked Winner" })],
    }));

    act(() => {
      source.emit("done", { rerank_applied: true });
    });
    await waitFor(() => expect(screen.getByTestId("refining")).toHaveTextContent("idle"));
    expect(source.closed).toBe(true);
    expect(screen.getByTestId("notice")).toHaveTextContent("none");
  });

  it("falls back to POST on a network failure with a notice that expires", async () => {
    const backend = installSearchBackend({ post: page("policy", "Fallback Result") });
    const harness = renderHarness(20);

    act(() => {
      void harness.controller.run(PAYLOAD);
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));

    act(() => {
      FakeEventSource.instances[0].failNetwork();
    });

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(harness.onPage).toHaveBeenCalledWith(expect.objectContaining({
      items: [expect.objectContaining({ title: "Fallback Result" })],
    }));
    expect(screen.getByTestId("notice")).toHaveTextContent("notice");

    await waitFor(() => expect(screen.getByTestId("notice")).toHaveTextContent("none"));
  });

  it("falls back to POST on a malformed frame instead of crashing", async () => {
    const backend = installSearchBackend({ post: page("policy", "Safe Fallback") });
    const harness = renderHarness();

    act(() => {
      void harness.controller.run(PAYLOAD);
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const source = FakeEventSource.instances[0];

    act(() => {
      source.emit("local", "not-json");
    });

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(harness.onPage).toHaveBeenCalledWith(expect.objectContaining({
      items: [expect.objectContaining({ title: "Safe Fallback" })],
    }));
    expect(source.closed).toBe(true);
  });

  it("falls back to POST on an in-band error frame", async () => {
    const backend = installSearchBackend({ post: page("policy", "Fallback Result") });
    const harness = renderHarness();

    act(() => {
      void harness.controller.run(PAYLOAD);
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));

    act(() => {
      FakeEventSource.instances[0].failFrame();
    });

    await waitFor(() => {
      expect(backend.callsFor("POST", "/v1/search")).toHaveLength(1);
    });
    expect(screen.getByTestId("notice")).toHaveTextContent("notice");
  });

  it("surfaces a POST failure to the caller after the stream fails", async () => {
    installSearchBackend({ postError: { status: 500, message: "search rejected" } });
    const harness = renderHarness();

    let caught: unknown = null;
    act(() => {
      void harness.controller.run(PAYLOAD).catch((error: unknown) => {
        caught = error;
      });
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));

    act(() => {
      FakeEventSource.instances[0].failNetwork();
    });

    await waitFor(() => expect(caught).not.toBeNull());
    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).toBe("search rejected");
    expect(screen.getByTestId("loading")).toHaveTextContent("idle");
  });

  it("closes the stream and aborts POST work on unmount", async () => {
    installSearchBackend();
    const harness = renderHarness();

    act(() => {
      void harness.controller.run(PAYLOAD);
    });
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    const source = FakeEventSource.instances[0];

    harness.unmount();
    expect(source.closed).toBe(true);
    expect(source.listenerCount("local")).toBe(0);
  });
});
