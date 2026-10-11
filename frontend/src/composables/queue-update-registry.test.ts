import { ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { TaskResponse } from "../services/api";
import { createQueueUpdateRegistry, queueUpdateRequiresReload, type QueueUpdateQuery } from "./queue-update-registry";

function task(taskId: string, status: TaskResponse["status"] = "running"): TaskResponse {
  return {
    task_id: taskId,
    kind: "file_batch",
    origin: "manual",
    status,
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
}

const query: QueueUpdateQuery = {
  query: "",
  view: "processing",
  status: null,
  kind: null,
  stage: null,
  waitingReason: null,
  dependencyKey: null,
  sort: { field: "created_at", direction: "desc" },
};

afterEach(() => vi.useRealTimers());

describe("queue update registry", () => {
  it("coalesces repeated task frames into one immutable page replacement at the fixed window edge", async () => {
    vi.useFakeTimers();
    const items = ref([task("one"), task("two")]);
    const original = items.value;
    const registry = createQueueUpdateRegistry({
      items,
      isLoading: () => false,
      onLoadingUpdate: vi.fn(),
      onStalePage: vi.fn(),
    });
    const first = { ...original[0]!, stage: "storage" };
    registry.enqueue(first);
    await vi.advanceTimersByTimeAsync(80);
    registry.enqueue({ ...first, progress: { ...first.progress, succeeded: 1 } });

    expect(registry.pendingCount).toBe(1);
    expect(items.value).toBe(original);
    await vi.advanceTimersByTimeAsync(19);
    expect(items.value).toBe(original);
    await vi.advanceTimersByTimeAsync(1);

    expect(items.value).not.toBe(original);
    expect(items.value[0]).not.toBe(original[0]);
    expect(items.value[0]?.stage).toBe("storage");
    expect(items.value[0]?.progress.succeeded).toBe(1);
    expect(items.value[1]).toBe(original[1]);
    expect(registry.pendingCount).toBe(0);
  });

  it("invalidates a queued page patch before a page replacement", async () => {
    vi.useFakeTimers();
    const oldPage = [task("old")];
    const items = ref(oldPage);
    const registry = createQueueUpdateRegistry({
      items,
      isLoading: () => false,
      onLoadingUpdate: vi.fn(),
      onStalePage: vi.fn(),
    });
    registry.enqueue({ ...oldPage[0]!, stage: "stale-update" });
    registry.invalidate();
    items.value = [task("new")];
    await vi.advanceTimersByTimeAsync(100);

    expect(items.value[0]?.task_id).toBe("new");
    expect(items.value[0]?.stage).toBe("processing");
    expect(registry.pendingCount).toBe(0);
  });

  it("falls back to an authoritative load if the page becomes busy or exceeds the buffer cap", async () => {
    vi.useFakeTimers();
    let loading = false;
    const onLoadingUpdate = vi.fn();
    const onStalePage = vi.fn();
    const items = ref([task("one")]);
    const registry = createQueueUpdateRegistry({
      items,
      isLoading: () => loading,
      onLoadingUpdate,
      onStalePage,
    });
    registry.enqueue({ ...items.value[0]!, stage: "queued-stage" });
    loading = true;
    await vi.advanceTimersByTimeAsync(100);
    expect(onLoadingUpdate).toHaveBeenCalledOnce();
    expect(items.value[0]?.stage).toBe("processing");

    loading = false;
    items.value = Array.from({ length: 101 }, (_, index) => task(`task-${index}`));
    for (const current of items.value.slice(0, 100)) registry.enqueue(current);
    expect(registry.pendingCount).toBe(100);
    expect(registry.enqueue(items.value[100]!)).toBe(false);
    expect(registry.pendingCount).toBe(0);
    expect(onStalePage).toHaveBeenCalledOnce();
  });
});

describe("queue update structural changes", () => {
  it("reloads for membership, selected filters, active sorts, and search rather than cloning server matching", () => {
    const current = task("one", "failed");
    expect(queueUpdateRequiresReload(current, { ...current, status: "succeeded" }, query)).toBe(true);
    expect(queueUpdateRequiresReload(current, { ...current, deleted_at: "2026-07-21T00:00:00Z" }, query)).toBe(true);
    expect(queueUpdateRequiresReload(current, { ...current, kind: "url_batch" }, { ...query, kind: "file_batch" })).toBe(true);
    expect(queueUpdateRequiresReload(current, { ...current, status: "running" }, { ...query, status: "failed" })).toBe(true);
    expect(queueUpdateRequiresReload(current, { ...current, updated_at: "2026-07-21T00:00:00Z" }, {
      ...query,
      sort: { field: "updated_at", direction: "desc" },
    })).toBe(true);
    expect(queueUpdateRequiresReload(current, { ...current, group_path: "archive" }, { ...query, query: "research" })).toBe(true);
    expect(queueUpdateRequiresReload(current, { ...current, progress: { ...current.progress, succeeded: 1 } }, query)).toBe(false);
  });
});
