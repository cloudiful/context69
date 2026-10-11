import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { useI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskPageResponse, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { FakeEventSource, installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { useProcessingQueue } from "./use-processing-queue";

const mocks = vi.hoisted(() => ({ addToast: vi.fn(), require: vi.fn() }));
vi.mock("@nuxt/ui/composables", () => ({ useToast: () => ({ add: mocks.addToast }) }));
vi.mock("./use-app-confirm", () => ({ useAppConfirm: () => ({ require: mocks.require }) }));

const listTasks = vi.spyOn(apiClient, "listTasks");

const runningTask: TaskResponse = {
  task_id: "running-task",
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
  updated_at: "2026-07-20T00:01:00Z",
};

function mountQueue(withDefaultResponse = true) {
  if (withDefaultResponse) {
    listTasks.mockImplementation(async () => ({
      items: [{ ...runningTask, progress: { ...runningTask.progress } }],
      pagination: { page: 1, page_size: 25, total: 1, total_pages: 1 },
    }) as never);
  }
  let state!: ReturnType<typeof useProcessingQueue>;
  const wrapper = mount(defineComponent({
    setup() {
      const { t } = useI18n();
      state = useProcessingQueue({ t });
      return {};
    },
    template: "<div />",
  }), { global: { plugins: [createTestI18n()] } });
  return { state, wrapper };
}

function setVisibility(value: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { value, configurable: true });
}

async function startStream(state: ReturnType<typeof useProcessingQueue>, previous: FakeEventSource[] = []) {
  state.startLiveUpdates();
  const stream = takeEventSource(previous);
  stream.emit("snapshot", { tasks: [runningTask] });
  await flushPromises();
  return stream;
}

beforeEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  setVisibility("visible");
  mocks.addToast.mockReset();
  mocks.require.mockReset();
  listTasks.mockReset();
  FakeEventSource.instances = [];
  FakeEventSource.failNextConstructor = false;
});
afterEach(() => vi.useRealTimers());

describe("queue stream update lifecycle", () => {
  it("clears buffered frames on a fresh snapshot and stream error", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountQueue();
    await flushPromises();
    const stream = await startStream(state);
    const callsAfterSnapshot = listTasks.mock.calls.length;
    const originalItems = state.items.value;
    vi.useFakeTimers();

    stream.emit("update", { task: { ...runningTask, stage: "stale-before-error" } });
    stream.emit("error", { message: "resync required" });
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(callsAfterSnapshot + 1);
    await vi.advanceTimersByTimeAsync(100);
    expect(state.items.value).not.toBe(originalItems);
    expect(state.items.value[0]?.stage).toBe("processing");

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("clears buffered updates when a terminal stream frame disconnects", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountQueue();
    await flushPromises();
    const stream = await startStream(state);
    const originalItems = state.items.value;
    vi.useFakeTimers();

    stream.emit("update", { task: { ...runningTask, stage: "must-not-patch-after-done" } });
    stream.emit("done", { tasks: [runningTask] });
    await vi.advanceTimersByTimeAsync(100);

    expect(state.items.value).toBe(originalItems);
    expect(state.items.value[0]?.stage).toBe("processing");
    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("turns updates during an in-flight snapshot into one trailing authoritative load", async () => {
    installFakeEventSource();
    const latest = { ...runningTask, task_id: "fresh-page-task" };
    let resolveInitial!: (value: TaskPageResponse) => void;
    listTasks
      .mockImplementationOnce(() => new Promise<TaskPageResponse>((resolve) => {
        resolveInitial = resolve;
      }) as never)
      .mockResolvedValueOnce({
        items: [latest],
        pagination: { page: 1, page_size: 25, total: 1, total_pages: 1 },
      } as never);
    const { state, wrapper } = mountQueue(false);
    await flushPromises();
    expect(state.loading.value).toBe(true);

    state.startLiveUpdates();
    const stream = takeEventSource();
    stream.emit("snapshot", { tasks: [runningTask] });
    stream.emit("update", { task: { ...runningTask, stage: "must-not-patch-stale-page" } });
    expect(listTasks).toHaveBeenCalledOnce();

    resolveInitial({
      items: [runningTask],
      pagination: { page: 1, page_size: 25, total: 1, total_pages: 1 },
    });
    await flushPromises();
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.items.value.map(({ task_id }) => task_id)).toEqual(["fresh-page-task"]);
    vi.useFakeTimers();
    await vi.advanceTimersByTimeAsync(100);
    expect(state.items.value[0]?.stage).toBe("processing");

    state.stopLiveUpdates();
    wrapper.unmount();
  });

  it("invalidates pending updates on stop, tab hiding, reconnect, and unmount", async () => {
    installFakeEventSource();
    const { state, wrapper } = mountQueue();
    await flushPromises();
    const first = await startStream(state);
    vi.useFakeTimers();

    let priorItems = state.items.value;
    first.emit("update", { task: { ...runningTask, stage: "stopped" } });
    state.stopLiveUpdates();
    await vi.advanceTimersByTimeAsync(100);
    expect(state.items.value).toBe(priorItems);

    const second = await startStream(state, [first]);
    priorItems = state.items.value;
    second.emit("update", { task: { ...runningTask, stage: "hidden" } });
    setVisibility("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(second.isClosed).toBe(true);
    await vi.advanceTimersByTimeAsync(100);
    expect(state.items.value).toBe(priorItems);

    setVisibility("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    const third = await startStream(state, [first, second]);
    priorItems = state.items.value;
    third.emit("update", { task: { ...runningTask, stage: "unmounted" } });
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(100);
    expect(state.items.value).toBe(priorItems);
  });
});
