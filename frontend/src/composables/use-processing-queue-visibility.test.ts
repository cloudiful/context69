import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { useProcessingQueue } from "./use-processing-queue";

const listTasks = vi.spyOn(apiClient, "listTasks");

const runningTask: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  origin: "manual",
  status: "running",
  group_path: "research",
  source_key: null,
  stage: "processing",
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

function setVisibility(value: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { value, configurable: true });
}

describe("useProcessingQueue hidden-tab SSE pause", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    setVisibility("visible");
    listTasks.mockReset().mockResolvedValue(page() as never);
  });

  it("disconnects the stream while hidden and reconnects with a snapshot resync on visible", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();

    state.startLiveUpdates();
    const first = takeEventSource();
    first.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();
    const loadsAfterSnapshot = listTasks.mock.calls.length;

    // Background tab: pause SSE so it costs zero event traffic.
    setVisibility("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(first.isClosed).toBe(true);

    // Foreground again: reconnect for a fresh snapshot; its triggered load
    // is the catch-up for updates missed while paused.
    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    const second = takeEventSource([first]);
    expect(second.isClosed).toBe(false);
    second.emit("snapshot", { tasks: [runningTask] });
    await flushPromises();
    expect(listTasks.mock.calls.length).toBeGreaterThan(loadsAfterSnapshot);

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("defers the stream open when live updates start while hidden", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountState();
    await flushPromises();

    setVisibility("hidden");
    state.startLiveUpdates();
    const { FakeEventSource } = await import("../test-utils/event-source");
    expect(FakeEventSource.instances).toHaveLength(0);

    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    const es = takeEventSource();
    expect(es.url).toContain("/v1/tasks/stream");

    state.stopLiveUpdates();
    wrapper.unmount();
  });
});
