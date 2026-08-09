import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
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

describe("createTaskSettler", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    getTask.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("polls until every task reaches a terminal status", async () => {
    getTask
      .mockResolvedValueOnce(taskState("task-1", "running") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results.map((state) => state.status)).toEqual(["succeeded"]);
    expect(getTask).toHaveBeenCalledTimes(2);
    expect(onTick).toHaveBeenCalledTimes(2);
    settler.dispose();
  });

  it("stops polling and refreshing after dispose", async () => {
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    settler.dispose();
    await vi.advanceTimersByTimeAsync(3000);
    const results = await pending;

    expect(results).toHaveLength(0);
    expect(onTick).not.toHaveBeenCalled();
    settler.dispose();
  });

  it("resolves a pending settle when disposed during the wait", async () => {
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(0);
    expect(onTick).toHaveBeenCalledTimes(1);

    settler.dispose();
    await pending;

    expect(onTick).toHaveBeenCalledTimes(1);
  });

  it("lets overlapping settles complete independently", async () => {
    getTask
      .mockResolvedValueOnce(taskState("task-1", "running") as never)
      .mockResolvedValueOnce(taskState("task-2", "running") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never)
      .mockResolvedValueOnce(taskState("task-2", "succeeded") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const first = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const second = settler.settle([{ task_id: "task-2", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    const [firstResults, secondResults] = await Promise.all([first, second]);

    expect(firstResults[0].status).toBe("succeeded");
    expect(secondResults[0].status).toBe("succeeded");
    expect(onTick).toHaveBeenCalledTimes(4);
    settler.dispose();
  });

  it("keeps polling when the refresh callback fails", async () => {
    getTask
      .mockResolvedValueOnce(taskState("task-1", "running") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never);
    const onTick = vi.fn()
      .mockRejectedValueOnce(new Error("network"))
      .mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    expect(onTick).toHaveBeenCalledTimes(2);
    settler.dispose();
  });

  it("keeps polling a task after a transient query failure", async () => {
    getTask
      .mockRejectedValueOnce(new Error("blip"))
      .mockResolvedValueOnce(taskState("task-1", "running") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    expect(getTask).toHaveBeenCalledTimes(3);
    settler.dispose();
  });

  it("drops a task after repeated query failures", async () => {
    getTask.mockRejectedValue(new Error("gone"));
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    await vi.advanceTimersByTimeAsync(1500);
    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results).toHaveLength(0);
    expect(getTask).toHaveBeenCalledTimes(4);
    expect(onTick).toHaveBeenCalledTimes(4);
    settler.dispose();
  });

  it("keeps polling while tasks stay active and skips further ticks once settled", async () => {
    getTask
      .mockResolvedValueOnce(taskState("task-1", "queued") as never)
      .mockResolvedValueOnce(taskState("task-1", "waiting") as never)
      .mockResolvedValueOnce(taskState("task-1", "succeeded") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const pending = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    await vi.advanceTimersByTimeAsync(1500);
    const results = await pending;

    expect(results[0].status).toBe("succeeded");
    expect(getTask).toHaveBeenCalledTimes(3);
    expect(onTick).toHaveBeenCalledTimes(3);
    settler.dispose();
  });
});
