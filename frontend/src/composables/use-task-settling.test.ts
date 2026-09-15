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

    expect(firstResults.map((state) => state.task_id)).toEqual(["task-1"]);
    expect(secondResults.map((state) => state.task_id)).toEqual(["task-2"]);
    expect(firstResults[0].status).toBe("succeeded");
    expect(secondResults[0].status).toBe("succeeded");
    expect(getTask).toHaveBeenCalledTimes(4);
    expect(onTick).toHaveBeenCalledTimes(2);
    settler.dispose();
  });

  it("merges concurrent settles into one tick per round", async () => {
    getTask.mockImplementation(async (taskId: string) => {
      const count = getTask.mock.calls.filter((call) => call[0] === taskId).length;
      if (taskId === "task-1") {
        return taskState(taskId, count >= 2 ? "succeeded" : "running") as never;
      }
      return taskState(taskId, count >= 2 ? "succeeded" : "running") as never;
    });
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const first = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const second = settler.settle([{ task_id: "task-2", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(1500);
    const [firstResults, secondResults] = await Promise.all([first, second]);

    expect(firstResults).toHaveLength(1);
    expect(secondResults).toHaveLength(1);
    expect(firstResults[0].task_id).toBe("task-1");
    expect(secondResults[0].task_id).toBe("task-2");
    expect(onTick).toHaveBeenCalledTimes(2);
    settler.dispose();
  });

  it("resolves each caller with only its own tasks as they finish", async () => {
    const calls = new Map<string, number>();
    getTask.mockImplementation(async (taskId: string) => {
      const next = (calls.get(taskId) ?? 0) + 1;
      calls.set(taskId, next);
      if (taskId === "task-1") return taskState(taskId, "succeeded") as never;
      return taskState(taskId, next >= 2 ? "succeeded" : "running") as never;
    });
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    let firstDone = false;
    let secondDone = false;
    const first = settler.settle([{ task_id: "task-1", item_ids: [] }]).then((results) => {
      firstDone = true;
      return results;
    });
    const second = settler.settle([{ task_id: "task-2", item_ids: [] }]).then((results) => {
      secondDone = true;
      return results;
    });

    await vi.advanceTimersByTimeAsync(0);
    expect(onTick).toHaveBeenCalledTimes(1);
    expect(firstDone).toBe(true);
    expect(secondDone).toBe(false);

    await vi.advanceTimersByTimeAsync(1500);
    const [firstResults, secondResults] = await Promise.all([first, second]);
    expect(firstResults.map((state) => state.task_id)).toEqual(["task-1"]);
    expect(secondResults.map((state) => state.task_id)).toEqual(["task-2"]);
    expect(onTick).toHaveBeenCalledTimes(2);
    settler.dispose();
  });

  it("counts the poll cap per task from its first join", async () => {
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    getTask.mockImplementation(async (taskId: string) => taskState(taskId, "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    let firstDone = false;
    let secondDone = false;
    const first = settler.settle([{ task_id: "task-1", item_ids: [] }]).then((results) => {
      firstDone = true;
      return results;
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(onTick).toHaveBeenCalledTimes(1);

    const second = settler.settle([{ task_id: "task-2", item_ids: [] }]).then((results) => {
      secondDone = true;
      return results;
    });

    await vi.advanceTimersByTimeAsync(119 * 1500);
    expect(firstDone).toBe(true);
    expect(secondDone).toBe(false);
    expect(onTick).toHaveBeenCalledTimes(120);

    await vi.advanceTimersByTimeAsync(1500);
    await Promise.all([first, second]);
    expect(firstDone).toBe(true);
    expect(secondDone).toBe(true);
    expect(onTick).toHaveBeenCalledTimes(121);
    expect(getTask.mock.calls.filter((call) => call[0] === "task-1")).toHaveLength(120);
    expect(getTask.mock.calls.filter((call) => call[0] === "task-2")).toHaveLength(120);
    settler.dispose();
  });

  it("resolves every concurrent waiter when disposed", async () => {
    getTask.mockResolvedValue(taskState("task-1", "running") as never);
    getTask.mockImplementation(async (taskId: string) => taskState(taskId, "running") as never);
    const onTick = vi.fn().mockResolvedValue(undefined);
    const settler = createTaskSettler(onTick);

    const first = settler.settle([{ task_id: "task-1", item_ids: [] }]);
    const second = settler.settle([{ task_id: "task-2", item_ids: [] }]);
    await vi.advanceTimersByTimeAsync(0);
    expect(onTick).toHaveBeenCalledTimes(1);

    settler.dispose();
    const [firstResults, secondResults] = await Promise.all([first, second]);
    expect(firstResults.map((state) => state.task_id)).toEqual(["task-1"]);
    expect(secondResults.map((state) => state.task_id)).toEqual(["task-2"]);

    await vi.advanceTimersByTimeAsync(3000);
    expect(onTick).toHaveBeenCalledTimes(1);
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
