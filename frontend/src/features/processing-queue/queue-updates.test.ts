import { describe, expect, it } from "vitest";

import type { TaskResponse } from "@/services/api";

import {
  applyTaskPatches,
  createPatchBuffer,
  queueUpdateRequiresReload,
  type QueueScope,
} from "./queue-updates";

function task(overrides: Partial<TaskResponse> = {}): TaskResponse {
  return {
    task_id: "task-1",
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: "research",
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    progress: { total: 2, queued: 0, running: 1, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
    ...overrides,
  };
}

function scope(overrides: Partial<QueueScope> = {}): QueueScope {
  return {
    view: "processing",
    page: 1,
    pageSize: 25,
    query: "",
    status: null,
    kind: null,
    stage: null,
    waitingReason: null,
    dependencyKey: null,
    sort: { field: "created_at", direction: "desc" },
    ...overrides,
  };
}

describe("queueUpdateRequiresReload", () => {
  it("treats a progress-only frame as patchable", () => {
    const current = task();
    const next = task({
      progress: { total: 2, queued: 0, running: 0, waiting: 0, succeeded: 2, failed: 0, cancelled: 0 },
      updated_at: "2026-01-01T00:01:00Z",
    });
    expect(queueUpdateRequiresReload(current, next, scope())).toBe(false);
  });

  it("reloads when the processing membership predicate flips", () => {
    const current = task({ status: "running" });
    expect(queueUpdateRequiresReload(current, task({ status: "succeeded" }), scope())).toBe(true);
  });

  it("reloads when a task leaves the completed view", () => {
    const current = task({ status: "succeeded" });
    const next = task({ status: "running" });
    expect(queueUpdateRequiresReload(current, next, scope({ view: "completed", sort: { field: "updated_at", direction: "desc" } }))).toBe(true);
  });

  it("reloads when a narrowing status filter no longer matches", () => {
    const current = task({ status: "failed" });
    const next = task({ status: "queued" });
    expect(queueUpdateRequiresReload(current, next, scope({ status: "failed" }))).toBe(true);
  });

  it("reloads on a stage, waiting reason or dependency change under a filter", () => {
    const current = task({ stage: "indexing", waiting_reason: "dependency", dependency_key: "qdrant" });
    expect(queueUpdateRequiresReload(current, task({ stage: "processing" }), scope({ stage: "indexing" }))).toBe(true);
    expect(queueUpdateRequiresReload(current, task({ waiting_reason: "backoff" }), scope({ waitingReason: "dependency" }))).toBe(true);
    expect(queueUpdateRequiresReload(current, task({ dependency_key: "s3" }), scope({ dependencyKey: "qdrant" }))).toBe(true);
  });

  it("reloads when the active sort field changes", () => {
    const current = task({ status: "queued" });
    const next = task({ status: "running" });
    expect(queueUpdateRequiresReload(current, next, scope({ sort: { field: "status", direction: "asc" } }))).toBe(true);
  });

  it("reloads for any frame while a free-text search is active", () => {
    const current = task();
    const next = task({ updated_at: "2026-01-01T00:02:00Z" });
    expect(queueUpdateRequiresReload(current, next, scope({ query: "  report  " }))).toBe(true);
  });

  it("reloads when the trash membership changes", () => {
    const current = task({ status: "failed" });
    expect(queueUpdateRequiresReload(current, task({ status: "failed", deleted_at: "2026-01-02T00:00:00Z" }), scope({ view: "trash" }))).toBe(true);
  });
});

describe("applyTaskPatches", () => {
  const window = [task({ task_id: "a" }), task({ task_id: "b" })];

  it("patches only the rows already in the window", () => {
    const result = applyTaskPatches(
      window,
      new Map([["b", task({ task_id: "b", status: "running", error_summary: "boom" })]]),
      scope(),
    );
    expect(result.kind).toBe("patch");
    if (result.kind !== "patch") return;
    expect(result.items).toHaveLength(2);
    expect(result.items[1]?.error_summary).toBe("boom");
    expect(result.items[0]).toBe(window[0]);
  });

  it("reports a frame for an unknown row as stale instead of inserting it", () => {
    const result = applyTaskPatches(window, new Map([["c", task({ task_id: "c" })]]), scope());
    expect(result.kind).toBe("stale");
  });

  it("reports a structural change as stale without mutating the window", () => {
    const result = applyTaskPatches(
      window,
      new Map([["a", task({ task_id: "a", status: "succeeded" })]]),
      scope(),
    );
    expect(result.kind).toBe("stale");
  });
});

describe("createPatchBuffer", () => {
  it("collapses repeated frames for one task to its latest state", () => {
    const buffer = createPatchBuffer();
    buffer.enqueue(task({ progress: { total: 2, queued: 0, running: 1, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 } }));
    buffer.enqueue(task({ progress: { total: 2, queued: 0, running: 0, waiting: 0, succeeded: 2, failed: 0, cancelled: 0 } }));
    expect(buffer.size).toBe(1);
    const flushed = buffer.flush();
    expect(flushed.size).toBe(1);
    expect(flushed.get("task-1")?.progress.succeeded).toBe(2);
    expect(buffer.size).toBe(0);
  });

  it("reports a full buffer so the caller can reload instead of dropping frames", () => {
    const buffer = createPatchBuffer(2);
    expect(buffer.enqueue(task({ task_id: "a" }))).toBe("buffered");
    expect(buffer.enqueue(task({ task_id: "b" }))).toBe("buffered");
    expect(buffer.enqueue(task({ task_id: "c" }))).toBe("full");
    expect(buffer.size).toBe(2);
  });

  it("drops buffered frames on invalidate", () => {
    const buffer = createPatchBuffer();
    buffer.enqueue(task());
    buffer.invalidate();
    expect(buffer.flush().size).toBe(0);
  });
});
