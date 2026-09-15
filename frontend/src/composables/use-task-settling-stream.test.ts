import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { FakeEventSource, installFakeEventSource, takeEventSource } from "../test-utils/event-source";
import { createTaskSettler } from "./use-task-settling";

const getTask = vi.spyOn(apiClient, "getTask");

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

describe("createTaskSettler task stream", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.unstubAllGlobals();
    getTask.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("opens the watch-ids stream with credentials and resolves terminal states from the snapshot", async () => {
    installFakeEventSource();
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const es = takeEventSource();
    expect(es.url).toContain("/v1/tasks/stream");
    expect(es.url).toContain("task_ids=task-1");
    expect(es.withCredentials).toBe(true);

    // The snapshot is the one full sync before deltas: a terminal snapshot
    // settles without waiting for the next poll.
    es.emit("snapshot", { tasks: [taskState("task-1", "succeeded")] });
    const results = await pending;

    expect(results.map((state) => state.status)).toEqual(["succeeded"]);
    expect(es.isClosed).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(onTick).toHaveBeenCalled();
    settler.dispose();
  });

  it("applies update deltas after the snapshot sync", async () => {
    installFakeEventSource();
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [taskState("task-1", "running")] });
    await vi.advanceTimersByTimeAsync(0);

    es.emit("update", { task: taskState("task-1", "succeeded") });
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    expect(es.isClosed).toBe(true);
    settler.dispose();
  });

  it("resolves from the done frame", async () => {
    installFakeEventSource();
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const es = takeEventSource();
    es.emit("snapshot", { tasks: [taskState("task-1", "running")] });
    es.emit("done", { tasks: [taskState("task-1", "succeeded")] });
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    expect(es.isClosed).toBe(true);
    settler.dispose();
  });

  it("keeps the D1 coalescing semantics: one shared stream, one tick per event, per-caller results", async () => {
    installFakeEventSource();
    // Hang the polling fallback so only stream ticks are observed.
    getTask.mockImplementation(() => new Promise(() => undefined) as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const first = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const second = settler.settle([{ task_id: "task-2", item_ids: [] }]);
    // Expanding the watched union reconnects so the new connection gets one
    // fresh snapshot (full sync) covering every pending task; the stale
    // connection is closed and a single shared stream remains.
    expect(FakeEventSource.instances).toHaveLength(2);
    expect(FakeEventSource.instances[0].isClosed).toBe(true);
    const es = takeEventSource(FakeEventSource.instances.slice(0, 1));
    expect(es.isClosed).toBe(false);
    expect(es.url).toContain("task-1");
    expect(es.url).toContain("task-2");

    let firstDone = false;
    let secondDone = false;
    const firstTracked = first.then((results) => {
      firstDone = true;
      return results;
    });
    const secondTracked = second.then((results) => {
      secondDone = true;
      return results;
    });

    es.emit("snapshot", { tasks: [taskState("task-1", "succeeded"), taskState("task-2", "running")] });
    await vi.advanceTimersByTimeAsync(0);
    expect(firstDone).toBe(true);
    expect(secondDone).toBe(false);

    es.emit("update", { task: taskState("task-2", "succeeded") });
    const [firstResults, secondResults] = await Promise.all([firstTracked, secondTracked]);
    expect(firstResults.map((state) => state.task_id)).toEqual(["task-1"]);
    expect(secondResults.map((state) => state.task_id)).toEqual(["task-2"]);
    // One tick per stream event (shared across callers), not per caller.
    await vi.advanceTimersByTimeAsync(0);
    expect(onTick).toHaveBeenCalledTimes(2);
    expect(es.isClosed).toBe(true);
    settler.dispose();
  });

  it("falls back to polling with zero behavior loss when the stream fails", async () => {
    installFakeEventSource();
    getTask
      .mockResolvedValueOnce(taskState("task-1", "running") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const es = takeEventSource();
    es.fail();

    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    expect(getTask).toHaveBeenCalled();
    expect(onTick).toHaveBeenCalled();
    settler.dispose();
  });

  it("falls back to polling when the stream constructor throws", async () => {
    installFakeEventSource();
    FakeEventSource.failNextConstructor = true;
    getTask
      .mockResolvedValueOnce(taskState("task-1", "running") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    expect(FakeEventSource.instances).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    settler.dispose();
  });

  it("closes the stream on dispose", async () => {
    installFakeEventSource();
    getTask.mockImplementation(() => new Promise(() => undefined) as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const es = takeEventSource();
    expect(es.isClosed).toBe(false);

    settler.dispose();
    expect(es.isClosed).toBe(true);
  });
});
