import { defineComponent, h } from "vue";
import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { FakeEventSource, installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { buildTaskStreamUrl, useTaskStream, type TaskStreamHooks } from "./use-task-stream";

function taskState(taskId: string, status: TaskResponse["status"]): TaskResponse {
  return {
    task_id: taskId,
    status,
    kind: "delete_batch",
    origin: "manual",
    progress: { succeeded: 0, failed: 0, total: 1, cancelled: 0, waiting: 0, queued: 0, running: 0 },
    created_at: "2026-08-01T00:00:00Z",
    updated_at: "2026-08-01T00:00:00Z",
  } as TaskResponse;
}

function makeHarness(hooks: TaskStreamHooks) {
  const Harness = defineComponent({
    setup() {
      return { stream: useTaskStream(hooks) };
    },
    render() {
      return h("div");
    },
  });
  const wrapper = mount(Harness, { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
  return {
    wrapper,
    streaming: wrapper.vm.stream.streaming,
    connected: wrapper.vm.stream.connected,
    usingFallback: wrapper.vm.stream.usingFallback,
    connect: wrapper.vm.stream.connect as (taskIds?: string[]) => void,
    disconnect: wrapper.vm.stream.disconnect as () => void,
  };
}

describe("buildTaskStreamUrl", () => {
  it("builds the watch-all URL without a query", () => {
    // resolveApiUrl prepends the origin when one is configured (jsdom), so
    // assert on the path suffix instead of the exact string.
    expect(buildTaskStreamUrl()).toContain("/v1/tasks/stream");
    expect(buildTaskStreamUrl()).not.toContain("task_ids");
    expect(buildTaskStreamUrl([])).not.toContain("task_ids");
  });

  it("encodes explicit task ids as a single task_ids param", () => {
    const url = buildTaskStreamUrl(["id-1", "id-2"]);
    expect(url).toContain("/v1/tasks/stream?");
    const params = new URLSearchParams(url.split("?")[1]);
    expect(params.get("task_ids")).toBe("id-1,id-2");
  });
});

describe("useTaskStream", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("opens EventSource with credentials and applies snapshot before deltas", () => {
    installFakeEventSource();
    const snapshots: TaskResponse[][] = [];
    const updates: TaskResponse[] = [];
    const harness = makeHarness({
      onSnapshot: (tasks) => snapshots.push(tasks),
      onUpdate: (task) => updates.push(task),
    });

    harness.connect(["task-1"]);
    const es = takeEventSource();
    expect(es.url).toContain("/v1/tasks/stream");
    expect(es.url).toContain("task_ids=task-1");
    expect(es.withCredentials).toBe(true);
    expect(harness.streaming.value).toBe(true);
    expect(harness.connected.value).toBe(false);

    // Reconnect-equivalent full sync first, then the incremental delta.
    es.emit("snapshot", { tasks: [taskState("task-1", "running")] });
    expect(harness.connected.value).toBe(true);
    es.emit("update", { task: taskState("task-1", "succeeded") });

    expect(snapshots).toHaveLength(1);
    expect(snapshots[0][0].status).toBe("running");
    expect(updates).toHaveLength(1);
    expect(updates[0].status).toBe("succeeded");
    expect(harness.usingFallback.value).toBe(false);
    harness.wrapper.unmount();
  });

  it("closes the stream after a done frame", () => {
    installFakeEventSource();
    const dones: TaskResponse[][] = [];
    const harness = makeHarness({
      onSnapshot: () => undefined,
      onUpdate: () => undefined,
      onDone: (tasks) => dones.push(tasks),
    });

    harness.connect(["task-1"]);
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [taskState("task-1", "running")] });
    es.emit("done", { tasks: [taskState("task-1", "succeeded")] });

    expect(dones).toHaveLength(1);
    expect(dones[0][0].status).toBe("succeeded");
    expect(es.isClosed).toBe(true);
    expect(harness.streaming.value).toBe(false);
    harness.wrapper.unmount();
  });

  it("falls back to polling on a network failure", () => {
    installFakeEventSource();
    const transportErrors = vi.fn();
    const harness = makeHarness({
      onSnapshot: () => undefined,
      onUpdate: () => undefined,
      onTransportError: transportErrors,
    });

    harness.connect();
    const es = takeEventSource();
    expect(es.url).toContain("/v1/tasks/stream");
    es.fail();

    expect(transportErrors).toHaveBeenCalledTimes(1);
    expect(harness.usingFallback.value).toBe(true);
    expect(harness.streaming.value).toBe(false);
    expect(harness.connected.value).toBe(false);
    expect(es.isClosed).toBe(true);
    harness.wrapper.unmount();
  });

  it("falls back to polling on an in-band error frame", () => {
    installFakeEventSource();
    const errorFrames: string[] = [];
    const transportErrors = vi.fn();
    const harness = makeHarness({
      onSnapshot: () => undefined,
      onUpdate: () => undefined,
      onErrorFrame: (message) => errorFrames.push(message),
      onTransportError: transportErrors,
    });

    harness.connect();
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [] });
    es.emit("error", { message: "task events lagged; resync via GET /v1/tasks and reconnect" });

    expect(errorFrames).toHaveLength(1);
    expect(errorFrames[0]).toContain("lagged");
    expect(transportErrors).toHaveBeenCalledTimes(1);
    expect(harness.usingFallback.value).toBe(true);
    expect(es.isClosed).toBe(true);
    harness.wrapper.unmount();
  });

  it("falls back to polling when the EventSource constructor throws", () => {
    installFakeEventSource();
    FakeEventSource.failNextConstructor = true;
    const transportErrors = vi.fn();
    const harness = makeHarness({
      onSnapshot: () => undefined,
      onUpdate: () => undefined,
      onTransportError: transportErrors,
    });

    harness.connect(["task-1"]);

    expect(transportErrors).toHaveBeenCalledTimes(1);
    expect(harness.usingFallback.value).toBe(true);
    expect(harness.streaming.value).toBe(false);
    expect(FakeEventSource.instances).toHaveLength(0);
    harness.wrapper.unmount();
  });

  it("falls back to polling when EventSource is unavailable (PAT clients)", () => {
    vi.stubGlobal("EventSource", undefined);
    const transportErrors = vi.fn();
    const harness = makeHarness({
      onSnapshot: () => undefined,
      onUpdate: () => undefined,
      onTransportError: transportErrors,
    });

    harness.connect();

    expect(transportErrors).toHaveBeenCalledTimes(1);
    expect(harness.usingFallback.value).toBe(true);
    expect(harness.streaming.value).toBe(false);
    harness.wrapper.unmount();
  });

  it("reconnect delivers a fresh snapshot before deltas", () => {
    installFakeEventSource();
    const snapshots: TaskResponse[][] = [];
    const updates: TaskResponse[] = [];
    const harness = makeHarness({
      onSnapshot: (tasks) => snapshots.push(tasks),
      onUpdate: (task) => updates.push(task),
    });

    harness.connect();
    const first = takeEventSource();
    first.emit("snapshot", { tasks: [taskState("task-1", "running")] });
    first.emit("update", { task: taskState("task-1", "running") });

    // Reconnect: the new connection must full-sync before any delta.
    harness.connect();
    expect(first.isClosed).toBe(true);
    const second = takeEventSource([first]);
    second.emit("snapshot", { tasks: [taskState("task-1", "succeeded")] });
    second.emit("update", { task: taskState("task-1", "succeeded") });

    expect(snapshots).toHaveLength(2);
    expect(snapshots[1][0].status).toBe("succeeded");
    expect(updates).toHaveLength(2);
    expect(harness.usingFallback.value).toBe(false);
    harness.wrapper.unmount();
  });

  it("disconnects and closes the stream on unmount", () => {
    installFakeEventSource();
    const harness = makeHarness({
      onSnapshot: () => undefined,
      onUpdate: () => undefined,
    });

    harness.connect();
    const es = takeEventSource();
    expect(harness.streaming.value).toBe(true);

    harness.disconnect();
    expect(es.isClosed).toBe(true);
    expect(harness.streaming.value).toBe(false);

    harness.connect();
    const reopened = takeEventSource([es]);
    harness.wrapper.unmount();
    expect(reopened.isClosed).toBe(true);
  });
});
