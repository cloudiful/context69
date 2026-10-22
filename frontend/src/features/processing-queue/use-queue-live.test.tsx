import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FakeEventSource, installFakeEventSource } from "@/test-utils/event-source";
import type { TaskResponse } from "@/services/api";

import type { QueueScope } from "./queue-updates";
import { LIVE_FALLBACK_INTERVAL_MS, STRUCTURAL_DEBOUNCE_MS, useQueueLive } from "./use-queue-live";

function task(overrides: Partial<TaskResponse> = {}): TaskResponse {
  return {
    task_id: "task-1",
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
    ...overrides,
  };
}

const scope: QueueScope = {
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
};

function Harness({
  enabled = true,
  patchResult = "patched" as "patched" | "stale",
  applyPatches,
  reload,
  isFetching = () => false,
  getAuthoritativeLanding = () => 0,
}: {
  enabled?: boolean;
  patchResult?: "patched" | "stale";
  applyPatches?: (updates: ReadonlyMap<string, TaskResponse>) => "patched" | "stale";
  reload?: () => void;
  isFetching?: () => boolean;
  getAuthoritativeLanding?: () => number;
}) {
  const state = useQueueLive({
    enabled,
    scope,
    applyPatches: applyPatches ?? (() => patchResult),
    reload: reload ?? (() => undefined),
    isFetching,
    getAuthoritativeLanding,
  });
  return <div data-testid="live-state">{state.fallback ? "fallback" : state.streaming ? "streaming" : "idle"}</div>;
}

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

describe("useQueueLive", () => {
  beforeEach(() => {
    installFakeEventSource();
    vi.useFakeTimers();
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("connects once and patches only known rows from a snapshot, then reloads", () => {
    const applyPatches = vi.fn((_updates: ReadonlyMap<string, TaskResponse>) => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} />);
    expect(FakeEventSource.instances).toHaveLength(1);

    act(() => {
      FakeEventSource.instances[0]!.emit("snapshot", { tasks: [task()] });
    });
    expect(applyPatches).toHaveBeenCalledWith(new Map([["task-1", expect.objectContaining({ task_id: "task-1" })]]));
    expect(reload).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("live-state").textContent).toBe("streaming");
  });

  it("ignores deltas that arrive before this connection's snapshot", () => {
    const applyPatches = vi.fn(() => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} />);
    const source = FakeEventSource.instances[0]!;

    act(() => {
      source.emit("update", { task: task() });
      vi.advanceTimersByTime(200);
    });
    expect(applyPatches).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();

    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    applyPatches.mockClear();
    act(() => {
      source.emit("update", { task: task() });
      vi.advanceTimersByTime(200);
    });
    expect(applyPatches).toHaveBeenCalledTimes(1);
  });

  it("coalesces progress frames into one patch without a reload", () => {
    const applyPatches = vi.fn((_updates: ReadonlyMap<string, TaskResponse>) => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} />);
    const source = FakeEventSource.instances[0]!;
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    applyPatches.mockClear();
    reload.mockClear();

    act(() => {
      source.emit("update", { task: task({ updated_at: "2026-01-01T00:00:01Z" }) });
      source.emit("update", { task: task({ updated_at: "2026-01-01T00:00:02Z" }) });
    });
    expect(applyPatches).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(applyPatches).toHaveBeenCalledTimes(1);
    expect(applyPatches.mock.calls[0]?.[0].size).toBe(1);
    expect(reload).not.toHaveBeenCalled();
  });

  it("queues one trailing authoritative reload for a structural burst", () => {
    const reload = vi.fn();
    render(<Harness patchResult="stale" reload={reload} />);
    const source = FakeEventSource.instances[0]!;
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    reload.mockClear();

    act(() => {
      source.emit("update", { task: task({ status: "succeeded" }) });
      vi.advanceTimersByTime(100);
      source.emit("update", { task: task({ status: "succeeded", task_id: "task-2" }) });
      vi.advanceTimersByTime(100);
    });
    expect(reload).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(STRUCTURAL_DEBOUNCE_MS);
    });
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("reloads immediately and drops buffered frames when a request is already in flight", () => {
    const applyPatches = vi.fn((_updates: ReadonlyMap<string, TaskResponse>) => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} isFetching={() => true} />);
    const source = FakeEventSource.instances[0]!;
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    reload.mockClear();
    applyPatches.mockClear();
    act(() => {
      source.emit("update", { task: task() });
    });
    expect(reload).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(applyPatches).not.toHaveBeenCalled();
  });

  it("drops a frame buffered before an authoritative landing without reloading", () => {
    let landing = 0;
    const applyPatches = vi.fn((_updates: ReadonlyMap<string, TaskResponse>) => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} getAuthoritativeLanding={() => landing} />);
    const source = FakeEventSource.instances[0]!;
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    applyPatches.mockClear();
    reload.mockClear();

    act(() => {
      source.emit("update", { task: task({ updated_at: "2026-01-01T00:00:02Z" }) });
    });
    // The authoritative read lands before the 100ms patch window closes.
    landing += 1;
    act(() => {
      vi.advanceTimersByTime(100);
    });

    expect(applyPatches).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();

    // A frame received after the landing is applied normally.
    act(() => {
      source.emit("update", { task: task({ updated_at: "2026-01-01T00:00:04Z" }) });
      vi.advanceTimersByTime(100);
    });
    expect(applyPatches).toHaveBeenCalledTimes(1);
    expect(applyPatches).toHaveBeenCalledWith(
      new Map([["task-1", expect.objectContaining({ updated_at: "2026-01-01T00:00:04Z" })]]),
    );
    expect(reload).not.toHaveBeenCalled();
  });

  it("keeps a fresh frame of the same burst and drops only the superseded one", () => {
    let landing = 0;
    const applyPatches = vi.fn((_updates: ReadonlyMap<string, TaskResponse>) => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} getAuthoritativeLanding={() => landing} />);
    const source = FakeEventSource.instances[0]!;
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    applyPatches.mockClear();
    reload.mockClear();

    act(() => {
      source.emit("update", { task: task({ task_id: "task-1", updated_at: "2026-01-01T00:00:02Z" }) });
    });
    landing += 1;
    act(() => {
      source.emit("update", { task: task({ task_id: "task-2", updated_at: "2026-01-01T00:00:03Z" }) });
      vi.advanceTimersByTime(100);
    });

    expect(applyPatches).toHaveBeenCalledTimes(1);
    expect([...applyPatches.mock.calls[0][0].keys()]).toEqual(["task-2"]);
    expect(reload).not.toHaveBeenCalled();
  });

  it("resets stamps together with the pending buffer when a snapshot resyncs", () => {
    let landing = 0;
    const applyPatches = vi.fn((_updates: ReadonlyMap<string, TaskResponse>) => "patched" as const);
    const reload = vi.fn();
    render(<Harness applyPatches={applyPatches} reload={reload} getAuthoritativeLanding={() => landing} />);
    const source = FakeEventSource.instances[0]!;
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    applyPatches.mockClear();
    reload.mockClear();

    act(() => {
      source.emit("update", { task: task({ updated_at: "2026-01-01T00:00:02Z" }) });
    });
    // A new snapshot supersedes the buffered frame and clears its stamp.
    act(() => {
      source.emit("snapshot", { tasks: [] });
    });
    applyPatches.mockClear();
    landing += 1;
    act(() => {
      vi.advanceTimersByTime(200);
    });

    // Only the snapshot's own (empty) patch reaches the window: the buffered
    // frame was dropped with its stamp.
    expect(applyPatches.mock.calls.every(([updates]) => updates.size === 0)).toBe(true);
  });

  it("starts the 20s poll only after transport failure and stops it on a fresh snapshot", () => {
    const reload = vi.fn();
    render(<Harness reload={reload} />);
    const source = FakeEventSource.instances[0]!;

    act(() => {
      source.fail();
    });
    expect(screen.getByTestId("live-state").textContent).toBe("fallback");
    act(() => {
      vi.advanceTimersByTime(LIVE_FALLBACK_INTERVAL_MS);
    });
    expect(reload).toHaveBeenCalledTimes(1);

    // A reconnect (tab visible again) is the only way to get a fresh snapshot
    // after the transport already failed.
    setVisibility("hidden");
    setVisibility("visible");
    act(() => {
      FakeEventSource.instances.at(-1)!.emit("snapshot", { tasks: [] });
    });
    expect(screen.getByTestId("live-state").textContent).toBe("streaming");
    const afterSnapshot = reload.mock.calls.length;
    act(() => {
      vi.advanceTimersByTime(LIVE_FALLBACK_INTERVAL_MS * 2);
    });
    expect(reload).toHaveBeenCalledTimes(afterSnapshot);
  });

  it("skips a fallback tick while a list request is fetching", () => {
    const reload = vi.fn();
    let fetching = false;
    render(<Harness reload={reload} isFetching={() => fetching} />);
    act(() => {
      FakeEventSource.instances[0]!.fail();
    });
    fetching = true;
    act(() => {
      vi.advanceTimersByTime(LIVE_FALLBACK_INTERVAL_MS);
    });
    expect(reload).not.toHaveBeenCalled();
  });

  it("does zero traffic in a hidden tab and resyncs when visible again", () => {
    const reload = vi.fn();
    render(<Harness reload={reload} />);
    const first = FakeEventSource.instances[0]!;

    setVisibility("hidden");
    expect(first.isClosed).toBe(true);
    act(() => {
      vi.advanceTimersByTime(LIVE_FALLBACK_INTERVAL_MS * 2);
    });
    expect(reload).not.toHaveBeenCalled();
    expect(FakeEventSource.instances).toHaveLength(1);

    setVisibility("visible");
    expect(FakeEventSource.instances).toHaveLength(2);
    act(() => {
      FakeEventSource.instances[1]!.emit("snapshot", { tasks: [] });
    });
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("closes everything on unmount", () => {
    const { unmount } = render(<Harness />);
    const source = FakeEventSource.instances[0]!;
    unmount();
    expect(source.isClosed).toBe(true);
  });

  it("does not connect when live updates are disabled", () => {
    render(<Harness enabled={false} />);
    expect(FakeEventSource.instances).toHaveLength(0);
    expect(screen.getByTestId("live-state").textContent).toBe("idle");
  });
});
