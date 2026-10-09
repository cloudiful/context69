import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import {
  FakeEventSource,
  installFakeEventSource,
  takeEventSource,
} from "../test-utils/event-source";
import { useProcessingQueue } from "./use-processing-queue";

const listTasks = vi.spyOn(apiClient, "listTasks");

const failedTask: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  origin: "manual",
  status: "failed",
  group_path: "research",
  source_key: null,
  stage: "indexing",
  waiting_reason: null,
  dependency_key: null,
  progress: {
    total: 1,
    queued: 0,
    running: 0,
    waiting: 0,
    succeeded: 0,
    failed: 1,
    cancelled: 0,
  },
  failure_stage: "indexing",
  error_summary: "Embedding failed",
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:01:00Z",
  finished_at: "2026-07-20T00:02:00Z",
  updated_at: "2026-07-20T00:02:00Z",
};

function page(items: TaskResponse[] = [failedTask]) {
  return {
    items,
    pagination: {
      page: 1,
      page_size: 25,
      total: items.length,
      total_pages: items.length ? 1 : 0,
    },
  };
}

function mountState() {
  let state!: ReturnType<typeof useProcessingQueue>;
  const wrapper = mount(
    defineComponent({
      setup() {
        state = useProcessingQueue({ t: (key) => key });
        return {};
      },
      template: "<div />",
    }),
    { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } },
  );
  return { state, wrapper };
}

describe("useProcessingQueue coalesced loading (issue 730)", () => {
  beforeEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    FakeEventSource.instances = [];
    FakeEventSource.failNextConstructor = false;
    listTasks.mockReset().mockResolvedValue(page() as never);
  });

  it("coalesces refresh triggers while a request is pending into one trailing snapshot", async () => {
    let resolveFirst!: (value: unknown) => void;
    listTasks
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve as (value: unknown) => void;
          }) as never,
      )
      .mockResolvedValueOnce(page([failedTask]) as never);
    const { state, wrapper } = mountState();
    // Initial mount load is pending.
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(1);
    expect(state.loading.value).toBe(true);

    // Two refresh triggers while pending coalesce instead of storming.
    state.refresh();
    state.refresh();
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(1);

    resolveFirst(page([failedTask]));
    await flushPromises();
    // Trailing snapshot runs once with the latest state.
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.loading.value).toBe(false);
    expect(state.items.value).toHaveLength(1);
    wrapper.unmount();
  });

  it("keeps the newest page when a page change races an in-flight request", async () => {
    const pageOneTask: TaskResponse = { ...failedTask, task_id: "page-1-task" };
    const pageTwoTask: TaskResponse = { ...failedTask, task_id: "page-2-task" };
    const pageFor = (pageNumber: number, items: TaskResponse[]) => ({
      items,
      pagination: {
        page: pageNumber,
        page_size: 25,
        total: 50,
        total_pages: 2,
      },
    });
    const seenPages: number[] = [];
    let resolveFirst!: (value: unknown) => void;
    listTasks
      .mockImplementationOnce(
        (params) =>
          new Promise((resolve) => {
            seenPages.push(params.page);
            resolveFirst = resolve as (value: unknown) => void;
          }) as never,
      )
      .mockImplementationOnce(
        (params) => {
          seenPages.push(params.page);
          return Promise.resolve(pageFor(2, [pageTwoTask]) as never);
        },
      );
    const { state, wrapper } = mountState();
    // Initial mount load requests page 1 and stays pending.
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(1);
    expect(seenPages).toEqual([1]);
    expect(state.loading.value).toBe(true);

    // A page change while pending queues a trailing snapshot for page 2.
    state.changePage(2);
    await flushPromises();
    expect(state.page.value).toBe(2);
    expect(listTasks).toHaveBeenCalledTimes(1);

    // The stale page-1 snapshot must not clobber the newer page ref; the
    // trailing reload observes page 2.
    resolveFirst(pageFor(1, [pageOneTask]));
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(seenPages).toEqual([1, 2]);
    expect(state.page.value).toBe(2);

    await flushPromises();
    expect(state.loading.value).toBe(false);
    expect(state.error.value).toBeNull();
    expect(state.page.value).toBe(2);
    expect(state.pagination.value.page).toBe(2);
    expect(state.items.value).toHaveLength(1);
    expect(state.items.value[0]?.task_id).toBe("page-2-task");
    wrapper.unmount();
  });

  it("settles a failed request with a user-visible error while preserving stale rows", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockRejectedValueOnce(new Error("boom"));
    const { state, wrapper } = mountState();
    await flushPromises();
    expect(state.items.value).toHaveLength(1);
    expect(state.error.value).toBeNull();

    state.refresh();
    await flushPromises();

    expect(state.loading.value).toBe(false);
    expect(state.error.value).toContain("boom");
    // Stale page stays usable instead of clearing to an indefinite spinner.
    expect(state.items.value).toHaveLength(1);
    wrapper.unmount();
  });

  it("bounds a hung request with a timeout that settles loading and reports an error", async () => {
    vi.useFakeTimers();
    listTasks.mockReset().mockImplementationOnce(
      (_params, options) =>
        new Promise((_resolve, reject) => {
          options?.signal?.addEventListener("abort", () => {
            const abortError = new Error("aborted");
            abortError.name = "AbortError";
            reject(abortError);
          });
        }) as never,
    );
    const { state, wrapper } = mountState();
    await flushPromises();
    expect(state.loading.value).toBe(true);

    await vi.advanceTimersByTimeAsync(15_000);
    await flushPromises();

    expect(state.loading.value).toBe(false);
    expect(state.error.value).toBe("processingQueue.loadFailed");
    state.stopLiveUpdates();
    wrapper.unmount();
    vi.useRealTimers();
  });

  it("keeps the fallback poll from overlapping an in-flight list request", async () => {
    installFakeEventSource();
    vi.useFakeTimers();
    Object.defineProperty(document, "visibilityState", {
      value: "visible",
      configurable: true,
    });
    // Mount settles, snapshot syncs, then the transport breaks to start the
    // 20s fallback.
    listTasks.mockReset().mockResolvedValue(page() as never);
    const { state, wrapper } = mountState();
    await flushPromises();
    state.startLiveUpdates();
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [] });
    await flushPromises();
    es.fail();
    await vi.advanceTimersByTimeAsync(0);
    await flushPromises();
    expect(state.liveFallback.value).toBe(true);
    const callsAfterFail = listTasks.mock.calls.length;

    // Hold the next list request open: the 20s fallback tick must skip while
    // loading is true instead of overlapping it.
    let resolveHung!: (value: unknown) => void;
    listTasks.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveHung = resolve as (value: unknown) => void;
        }) as never,
    );
    state.refresh();
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(callsAfterFail + 1);
    expect(state.loading.value).toBe(true);

    await vi.advanceTimersByTimeAsync(20_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(callsAfterFail + 1);

    resolveHung(page());
    await flushPromises();
    await vi.advanceTimersByTimeAsync(20_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBeGreaterThan(callsAfterFail + 1);

    state.stopLiveUpdates();
    wrapper.unmount();
    vi.useRealTimers();
  });
});
