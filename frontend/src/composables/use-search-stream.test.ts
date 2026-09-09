import { defineComponent, h } from "vue";
import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type SearchRequest, type SearchResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { FakeEventSource, installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { useSearchStream, type SearchStreamHooks } from "./use-search-stream";

function searchResponse(query: string, title: string): SearchResponse {
  return {
    query,
    items: [
      {
        chunk_id: `c-${title}`,
        document_id: 1,
        source_key: "src",
        external_id: "e",
        group_key: "g",
        group_path: "g/p",
        visibility: "private",
        title,
        summary: "",
        source_uri: "https://example.com/x",
        published_at: null,
        chunk_index: 0,
        chunk_text: `${title} body`,
        score: 0.5,
        metadata_json: {},
      },
    ],
    pagination: { page: 1, page_size: 8, total: 1, total_pages: 1, has_more: false },
  };
}

function makeHarness(hooks: SearchStreamHooks) {
  const Harness = defineComponent({
    setup() {
      return { stream: useSearchStream(hooks) };
    },
    render() {
      return h("div");
    },
  });
  const wrapper = mount(Harness, { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
  return {
    wrapper,
    loading: wrapper.vm.stream.loading,
    refining: wrapper.vm.stream.refining,
    fallbackNotice: wrapper.vm.stream.fallbackNotice,
    run: wrapper.vm.stream.run as (payload: SearchRequest) => Promise<void>,
  };
}

describe("useSearchStream", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("uses POST directly for cursor-bearing requests", async () => {
    const commits: SearchResponse[] = [];
    const search = vi.spyOn(apiClient, "search").mockResolvedValue(searchResponse("q", "Post"));
    const harness = makeHarness({ onResults: (page) => commits.push(page) });

    await harness.run({ query: "q", limit: 8, page: 1, cursor: "c1:8" });
    expect(search).toHaveBeenCalledTimes(1);
    expect(commits).toHaveLength(1);
    expect(commits[0].items[0].title).toBe("Post");
    expect(FakeEventSource.instances).toHaveLength(0);
    harness.wrapper.unmount();
  });

  it("closes the previous stream when a new search starts", async () => {
    installFakeEventSource();
    const commits: SearchResponse[] = [];
    const harness = makeHarness({ onResults: (page) => commits.push(page) });
    const post = vi.spyOn(apiClient, "search").mockResolvedValue(searchResponse("q", "Post"));

    const firstRun = harness.run({ query: "first", limit: 8, page: 1 });
    expect(FakeEventSource.instances).toHaveLength(1);
    const first = FakeEventSource.instances[0];
    expect(first.isClosed).toBe(false);

    // A new search aborts the first connection before it completes.
    const secondRun = harness.run({ query: "second", limit: 8, page: 1 });
    expect(first.isClosed).toBe(true);
    expect(FakeEventSource.instances).toHaveLength(2);
    const second = FakeEventSource.instances[1];
    expect(second.isClosed).toBe(false);

    second.emit("local", {
      items: [],
      pagination: { page: 1, page_size: 8, total: 0, total_pages: 0, has_more: false },
    });
    second.emit("done", { rerank_applied: false });
    await firstRun;
    await secondRun;
    expect(post).not.toHaveBeenCalled();
    expect(commits).toHaveLength(1);
    harness.wrapper.unmount();
  });

  it("aborts the stream and flags loading off on unmount", async () => {
    installFakeEventSource();
    const harness = makeHarness({ onResults: () => undefined });
    const runPromise = harness.run({ query: "q", limit: 8, page: 1 });
    expect(FakeEventSource.instances).toHaveLength(1);
    const es = FakeEventSource.instances[0];
    expect(harness.loading.value).toBe(true);

    harness.wrapper.unmount();
    expect(es.isClosed).toBe(true);
    expect(harness.loading.value).toBe(false);
    await runPromise;
  });

  it("falls back to POST on a network failure with a non-blocking notice", async () => {
    installFakeEventSource();
    const commits: SearchResponse[] = [];
    const harness = makeHarness({ onResults: (page) => commits.push(page) });
    const post = vi.spyOn(apiClient, "search").mockResolvedValue(searchResponse("q", "Fallback"));

    const runPromise = harness.run({ query: "q", limit: 8, page: 1 });
    await Promise.resolve();
    const es = takeEventSource();
    es.fail();
    await runPromise;

    expect(post).toHaveBeenCalledTimes(1);
    expect(commits).toHaveLength(1);
    expect(commits[0].items[0].title).toBe("Fallback");
    expect(harness.fallbackNotice.value).toBe(true);
    expect(harness.loading.value).toBe(false);
    harness.wrapper.unmount();
  });

  it("honours an in-band error frame by switching to POST", async () => {
    installFakeEventSource();
    const commits: SearchResponse[] = [];
    const harness = makeHarness({ onResults: (page) => commits.push(page) });
    const post = vi.spyOn(apiClient, "search").mockResolvedValue(searchResponse("q", "After Error"));

    const runPromise = harness.run({ query: "q", limit: 8, page: 1 });
    await Promise.resolve();
    const es = takeEventSource();
    es.emit("local", {
      items: [],
      pagination: { page: 1, page_size: 8, total: 0, total_pages: 0, has_more: false },
    });
    es.emit("error", { message: "cursor ordering mismatch" });
    await runPromise;

    expect(post).toHaveBeenCalledTimes(1);
    expect(commits.at(-1)?.items[0].title).toBe("After Error");
    harness.wrapper.unmount();
  });
});
