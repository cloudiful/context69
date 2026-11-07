import { describe, expect, it, vi } from "vitest";

import { ApiError, unwrapResponse } from "./api-core";
import { createTasksApi } from "./api-tasks";
import type { TaskDiagnoseResponse } from "./api-types";

function diagnoseResponse(): TaskDiagnoseResponse {
  return {
    task: {
      task_id: "task-1",
      kind: "file_batch",
      status: "waiting",
      progress: { total: 1, queued: 0, running: 0, waiting: 1, succeeded: 0, failed: 0, cancelled: 0 },
      stage: "processing",
      waiting_reason: "dependency",
      dependency_key: "s3",
      next_attempt_at: "2026-07-20T00:10:00Z",
      failure_stage: null,
      error_summary: null,
      lease_expires_at: "2026-07-20T00:05:00Z",
      created_at: "2026-07-20T00:01:00Z",
      started_at: "2026-07-20T00:01:00Z",
      finished_at: null,
      updated_at: "2026-07-20T00:02:00Z",
    },
    items: [
      {
        item_id: "item-1",
        ordinal: 0,
        status: "waiting",
        stage: "processing",
        waiting_reason: "dependency",
        dependency_key: "s3",
        next_attempt_at: "2026-07-20T00:10:00Z",
        failure_stage: null,
        error_message: null,
        attempt_count: 2,
        retryable: true,
        lease_expires_at: null,
        active_attempt: null,
        latest_attempt: null,
        created_at: "2026-07-20T00:01:00Z",
        started_at: "2026-07-20T00:01:00Z",
        finished_at: null,
      },
    ],
    items_truncated: false,
    dependency_gates: [],
    consistency: {
      consistent: true,
      current_item_id: "item-1",
      mismatches: [],
      open_attempt_count: 0,
      near_exhaustion_item_count: 0,
    },
    observed_at: "2026-07-20T00:02:00Z",
  };
}

function createApi(get: ReturnType<typeof vi.fn>, post: ReturnType<typeof vi.fn> = vi.fn()) {
  return createTasksApi({
    openapiClient: { GET: get, POST: post, DELETE: vi.fn() } as never,
    unwrapResponse,
  });
}

describe("tasks api diagnose", () => {
  it("reads the diagnose projection for one task with its path param", async () => {
    const data = diagnoseResponse();
    const get = vi.fn().mockResolvedValue({ data, response: { ok: true, status: 200 } });
    const api = createApi(get);

    const result = await api.getTaskDiagnose("task-1");

    expect(get).toHaveBeenCalledWith("/v1/tasks/{task_id}/diagnose", {
      params: { path: { task_id: "task-1" } },
      signal: undefined,
    });
    expect(result).toBe(data);
  });

  it("forwards an abort signal so a stale diagnose request can be cancelled", async () => {
    const controller = new AbortController();
    const get = vi.fn().mockResolvedValue({ data: diagnoseResponse(), response: { ok: true, status: 200 } });
    const api = createApi(get);

    await api.getTaskDiagnose("task-1", { signal: controller.signal });

    expect(get).toHaveBeenCalledWith(
      "/v1/tasks/{task_id}/diagnose",
      expect.objectContaining({ signal: controller.signal }),
    );
  });

  it("throws a typed ApiError with the upstream message when diagnose fails", async () => {
    const get = vi.fn().mockResolvedValue({
      error: { message: "task not found" },
      response: { ok: false, status: 404 },
    });
    const api = createApi(get);

    await expect(api.getTaskDiagnose("missing")).rejects.toMatchObject({
      name: "ApiError",
      message: "task not found",
      status: 404,
    });
    await expect(api.getTaskDiagnose("missing")).rejects.toBeInstanceOf(ApiError);
  });
});

describe("tasks api pause", () => {
  it("requests a pause with the task path param and returns the refreshed task", async () => {
    const data = { ...diagnoseResponse().task, pause_state: "paused", paused_at: "2026-07-20T00:03:00Z" };
    const post = vi.fn().mockResolvedValue({ data, response: { ok: true, status: 200 } });
    const api = createApi(vi.fn(), post);

    const result = await api.pauseTask("task-1");

    expect(post).toHaveBeenCalledWith("/v1/tasks/{task_id}/pause", {
      params: { path: { task_id: "task-1" } },
      signal: undefined,
    });
    expect(result).toBe(data);
  });

  it("requests a resume with the task path param", async () => {
    const post = vi.fn().mockResolvedValue({ data: {}, response: { ok: true, status: 200 } });
    const api = createApi(vi.fn(), post);

    await api.resumeTask("task-1", { signal: undefined });

    expect(post).toHaveBeenCalledWith("/v1/tasks/{task_id}/resume", {
      params: { path: { task_id: "task-1" } },
      signal: undefined,
    });
  });
});
