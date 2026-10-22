import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FakeEventSource, installFakeEventSource } from "@/test-utils/event-source";
import type { TaskResponse } from "@/services/api";

import { buildTaskStreamUrl, createTaskStream } from "./task-stream";

function task(taskId = "task-1"): TaskResponse {
  return {
    task_id: taskId,
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: null,
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    progress: { total: 1, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
  };
}

describe("buildTaskStreamUrl", () => {
  it("builds the watch-all URL without a task id list", () => {
    expect(buildTaskStreamUrl()).toMatch(/\/v1\/tasks\/stream$/);
    expect(buildTaskStreamUrl([])).toMatch(/\/v1\/tasks\/stream$/);
  });

  it("adds the explicit watch list as one query parameter", () => {
    const url = buildTaskStreamUrl([" a ", "b", ""]);
    expect(url).toContain("task_ids=a%2Cb");
  });
});

describe("createTaskStream", () => {
  beforeEach(() => {
    installFakeEventSource();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function build(overrides: Partial<Parameters<typeof createTaskStream>[0]> = {}) {
    const hooks = {
      onSnapshot: vi.fn(),
      onUpdate: vi.fn(),
      onDone: vi.fn(),
      onErrorFrame: vi.fn(),
      onTransportError: vi.fn(),
      ...overrides,
    };
    return { hooks, controller: createTaskStream(hooks) };
  }

  it("connects with cookies and no Last-Event-ID", () => {
    const { controller } = build();
    controller.connect();
    const source = FakeEventSource.instances[0]!;
    expect(source.withCredentials).toBe(true);
    expect(source.url).not.toContain("Last-Event-ID");
    expect(source.isClosed).toBe(false);
  });

  it("delivers snapshot, update and done frames", () => {
    const { hooks, controller } = build();
    controller.connect();
    const source = FakeEventSource.instances[0]!;
    source.emit("snapshot", { tasks: [task("a")] });
    source.emit("update", { task: task("b") });
    source.emit("done", { tasks: [task("c")] });
    expect(hooks.onSnapshot).toHaveBeenCalledWith([expect.objectContaining({ task_id: "a" })]);
    expect(hooks.onUpdate).toHaveBeenCalledWith(expect.objectContaining({ task_id: "b" }));
    expect(hooks.onDone).toHaveBeenCalledWith([expect.objectContaining({ task_id: "c" })]);
  });

  it("ignores malformed frames instead of throwing", () => {
    const { hooks, controller } = build();
    controller.connect();
    const source = FakeEventSource.instances[0]!;
    const malformed = { data: "{not json" } as unknown as Event;
    source.addEventListener("snapshot", () => undefined);
    // Drive the registered listeners directly with invalid JSON.
    (source as unknown as { listeners: Map<string, ((event: Event) => void)[]> })
      .listeners.get("snapshot")?.forEach((listener) => listener(malformed));
    (source as unknown as { listeners: Map<string, ((event: Event) => void)[]> })
      .listeners.get("update")?.forEach((listener) => listener(malformed));
    expect(hooks.onSnapshot).not.toHaveBeenCalled();
    expect(hooks.onUpdate).not.toHaveBeenCalled();
  });

  it("reports a transport failure once and closes the stream", () => {
    const { hooks, controller } = build();
    controller.connect();
    const source = FakeEventSource.instances[0]!;
    source.fail();
    expect(hooks.onTransportError).toHaveBeenCalledTimes(1);
    expect(source.isClosed).toBe(true);
    expect(controller.isOpen()).toBe(false);
  });

  it("reports a named error frame with its message", () => {
    const { hooks, controller } = build();
    controller.connect();
    const source = FakeEventSource.instances[0]!;
    source.emit("error", { message: "stream disabled" });
    expect(hooks.onErrorFrame).toHaveBeenCalledWith("stream disabled");
    expect(hooks.onTransportError).toHaveBeenCalledTimes(1);
    expect(source.isClosed).toBe(true);
  });

  it("detaches every named listener before closing", () => {
    const { controller } = build();
    controller.connect();
    const source = FakeEventSource.instances[0]!;
    const listeners = (source as unknown as { listeners: Map<string, unknown[]> }).listeners;
    expect(listeners.get("snapshot")?.length).toBe(1);
    controller.disconnect();
    expect(source.isClosed).toBe(true);
    expect(listeners.size).toBe(0);
  });

  it("reconnects with a fresh source instead of replaying the old one", () => {
    const { hooks, controller } = build();
    controller.connect();
    const first = FakeEventSource.instances[0]!;
    controller.reconnect();
    expect(first.isClosed).toBe(true);
    const second = FakeEventSource.instances[1]!;
    second.emit("snapshot", { tasks: [] });
    expect(hooks.onSnapshot).toHaveBeenCalledTimes(1);
    // A late frame from the closed source is ignored.
    first.emit("snapshot", { tasks: [task()] });
    expect(hooks.onSnapshot).toHaveBeenCalledTimes(1);
  });

  it("falls back to polling when EventSource cannot be constructed", () => {
    FakeEventSource.failNextConstructor = true;
    const { hooks, controller } = build();
    controller.connect();
    expect(hooks.onTransportError).toHaveBeenCalledTimes(1);
  });
});
