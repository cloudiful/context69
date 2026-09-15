import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { FakeEventSource, installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { useProcessingQueue } from "./use-processing-queue";

const listTasks = vi.spyOn(apiClient, "listTasks");

const runningTask: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  origin: "manual",
  status: "running",
  group_path: "research",
  source_key: null,
  stage: "indexing",
  waiting_reason: null,
  dependency_key: null,
  progress: { total: 1, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:01:00Z",
  finished_at: null,
  updated_at: "2026-07-20T00:02:00Z",
};

function page(items: TaskResponse[] = [runningTask]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

function mountState() {
  let state!: ReturnType<typeof useProcessingQueue>;
  const wrapper = mount(defineComponent({
    setup() {
      state = useProcessingQueue({ t: (key) => key });
      return {};
    },
    template: "<div />",
  }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
  return { state, wrapper };
}

describe("useProcessingQueue live updates", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    FakeEventSource.instances = [];
    FakeEventSource.failNextConstructor = false;
    listTasks.mockReset().mockResolvedValue(page() as never);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("drives refreshes from the watch-all stream snapshot and merges updates", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(1);

    state.startLiveUpdates();
    const es = takeEventSource();
    expect(es.url).toContain("/v1/tasks/stream");
    expect(es.url).not.toContain("task_ids");
    expect(es.withCredentials).toBe(true);

    es.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(2);

    // Issue 408 Task F1: an in-page update merges locally with zero requests.
    es.emit("update", { task: { ...runningTask, status: "succeeded" } });
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.items.value).toHaveLength(1);
    expect(state.items.value[0]?.status).toBe("succeeded");
    expect(state.liveFallback.value).toBe(false);

    state.stopLiveUpdates();
    expect(es.isClosed).toBe(true);
    wrapper.unmount();
  });

  it("falls back to the retained 20s poll when the stream fails, keeping filters", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setStatusFilter("failed");
    await flushPromises();
    const callsBeforeStream = listTasks.mock.calls.length;

    state.startLiveUpdates();
    const es = takeEventSource();
    expect(state.liveFallback.value).toBe(false);

    // Establish the stream with its full-sync snapshot first.
    es.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();

    vi.useFakeTimers();
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    es.fail();
    expect(state.liveFallback.value).toBe(true);
    expect(es.isClosed).toBe(true);

    // Breaking an established stream resyncs once with the active filter intact.
    await vi.advanceTimersByTimeAsync(0);
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "failed" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    const callsAfterFail = listTasks.mock.calls.length;
    expect(callsAfterFail).toBeGreaterThan(callsBeforeStream);

    // The retained poll keeps refreshing until live updates stop.
    await vi.advanceTimersByTimeAsync(20_000);
    expect(listTasks.mock.calls.length).toBeGreaterThan(callsAfterFail);
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "failed" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.stopLiveUpdates();
    const callsAfterStop = listTasks.mock.calls.length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(listTasks.mock.calls.length).toBe(callsAfterStop);
    wrapper.unmount();
  });

  it("starts the polling fallback without throwing when EventSource is unavailable", async () => {
    vi.stubGlobal("EventSource", undefined);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(() => state.startLiveUpdates()).not.toThrow();
    expect(state.liveFallback.value).toBe(true);
    expect(FakeEventSource.instances).toHaveLength(0);

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("merges an in-page storm with zero requests and keeps counts realtime", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();
    state.startLiveUpdates();
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();
    const baseline = listTasks.mock.calls.length;

    vi.useFakeTimers();
    for (let i = 0; i < 50; i += 1) {
      es.emit("update", {
        task: {
          ...runningTask,
          status: i === 49 ? "failed" : "running",
          stage: `stage-${i}`,
          progress: { total: 50, queued: 0, running: 50 - i - 1, waiting: 0, succeeded: i, failed: 0, cancelled: 0 },
          error_summary: i === 49 ? "boom" : null,
          failure_stage: i === 49 ? "indexing" : null,
        },
      });
    }
    await vi.advanceTimersByTimeAsync(5_000);
    await flushPromises();

    expect(listTasks.mock.calls.length).toBe(baseline);
    expect(state.items.value).toHaveLength(1);
    expect(state.items.value[0]?.status).toBe("failed");
    expect(state.items.value[0]?.stage).toBe("stage-49");
    expect(state.items.value[0]?.progress.succeeded).toBe(49);
    expect(state.items.value[0]?.error_summary).toBe("boom");
    expect(state.failedCount.value).toBe(1);

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("collapses an off-page storm into one trailing debounced load", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();
    state.startLiveUpdates();
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();
    const baseline = listTasks.mock.calls.length;

    vi.useFakeTimers();
    for (let i = 0; i < 50; i += 1) {
      es.emit("update", { task: { ...runningTask, task_id: `off-page-${i}` } });
    }
    // No synchronous reload: the burst is still inside the debounce window.
    expect(listTasks.mock.calls.length).toBe(baseline);

    await vi.advanceTimersByTimeAsync(1_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(baseline + 1);

    // The window drained: idling fires no further loads.
    await vi.advanceTimersByTimeAsync(10_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(baseline + 1);

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("merges staggered off-page frames inside one window into a single load", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();
    state.startLiveUpdates();
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();
    const baseline = listTasks.mock.calls.length;

    vi.useFakeTimers();
    es.emit("update", { task: { ...runningTask, task_id: "off-page-a" } });
    await vi.advanceTimersByTimeAsync(400);
    es.emit("update", { task: { ...runningTask, task_id: "off-page-b" } });
    await vi.advanceTimersByTimeAsync(400);
    es.emit("update", { task: { ...runningTask, task_id: "off-page-c" } });
    await vi.advanceTimersByTimeAsync(400);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(baseline + 1);

    await vi.advanceTimersByTimeAsync(5_000);
    await flushPromises();
    expect(listTasks.mock.calls.length).toBe(baseline + 1);

    state.stopLiveUpdates();
    wrapper.unmount();
  });
});
