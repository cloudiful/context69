import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, ref } from "vue";

import {
  ITEM_REFRESH_INTERVAL_MS,
  useTaskItemsRefresh,
  type UseTaskItemsRefreshOptions,
} from "./use-task-items-refresh";

// The composable owns lifecycle hooks, so it is exercised inside a host
// component that mirrors how the expanded panel uses it.
function mountRefresh(options: {
  taskId?: string;
  enabled?: boolean;
  intervalMs?: number;
  refresh: () => Promise<void>;
}) {
  const state = ref(options.taskId ?? "task-a");
  const enabled = ref(options.enabled ?? true);
  const exposed: { value: ReturnType<typeof useTaskItemsRefresh> | null } = { value: null };

  const Host = defineComponent({
    setup() {
      const opts: UseTaskItemsRefreshOptions = {
        taskId: state,
        enabled,
        refresh: options.refresh,
        intervalMs: options.intervalMs ?? ITEM_REFRESH_INTERVAL_MS,
      };
      exposed.value = useTaskItemsRefresh(opts);
      return () => h("div");
    },
  });

  const wrapper = mount(Host);
  return {
    wrapper,
    taskId: state,
    enabled,
    api: exposed.value!,
  };
}

describe("useTaskItemsRefresh", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reloads on its own cadence while an active task is mounted", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const { api, wrapper } = mountRefresh({ refresh });

    // The parent task's updated_at never moves mid-run, so the timer is the
    // only thing that can surface item progress during a long ingest.
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS * 3);

    expect(refresh).toHaveBeenCalledTimes(3);
    wrapper.unmount();
  });

  it("never overlaps requests: a tick during an in-flight reload is dropped", async () => {
    vi.useFakeTimers();
    let resolveRefresh: (() => void) | null = null;
    const refresh = vi.fn().mockImplementation(
      () => new Promise<void>((resolve) => { resolveRefresh = resolve; }),
    );
    const { api, wrapper } = mountRefresh({ refresh });

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(api.isRefreshing.value).toBe(true);

    // Two more intervals pass while the first request is still open.
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS * 2);
    expect(refresh).toHaveBeenCalledTimes(1);

    resolveRefresh!();
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(refresh).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("serializes manual item fetches behind periodic refreshes", async () => {
    vi.useFakeTimers();
    let resolveRefresh: (() => void) | null = null;
    const refresh = vi.fn().mockImplementation(
      () => new Promise<void>((resolve) => { resolveRefresh = resolve; }),
    );
    const { api, wrapper } = mountRefresh({ refresh });
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    const manualRequest = vi.fn().mockResolvedValue(undefined);
    const manualResult = api.runExclusive(manualRequest);

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(manualRequest).not.toHaveBeenCalled();

    resolveRefresh!();
    expect(await manualResult).toBe(true);
    expect(manualRequest).toHaveBeenCalledTimes(1);
    expect(api.isRefreshing.value).toBe(false);
    wrapper.unmount();
  });

  it("rejects queued work after the host component unmounts", async () => {
    let resolveActive: (() => void) | null = null;
    const refresh = vi.fn().mockResolvedValue(undefined);
    const { api, wrapper } = mountRefresh({ refresh });
    const active = api.runExclusive(
      () => new Promise<void>((resolve) => { resolveActive = resolve; }),
    );
    await Promise.resolve();
    const queuedRequest = vi.fn().mockResolvedValue(undefined);
    const queued = api.runExclusive(queuedRequest);

    wrapper.unmount();
    resolveActive!();

    expect(await active).toBe(false);
    expect(await queued).toBe(false);
    expect(queuedRequest).not.toHaveBeenCalled();
  });

  it("keeps the last valid data and reports a retryable failure instead of clearing it", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn().mockRejectedValue(new Error("network down"));
    const { api, wrapper } = mountRefresh({ refresh });

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);

    // A transient failure must not be destructive: the caller's data stays and
    // only a non-blocking notice appears.
    expect(api.refreshError.value).toBe(true);
    expect(api.isRefreshing.value).toBe(false);
    wrapper.unmount();
  });

  it("clears the failure notice once a later reload succeeds", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValue(undefined);
    const { api, wrapper } = mountRefresh({ refresh });

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(api.refreshError.value).toBe(true);

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(api.refreshError.value).toBe(false);
    wrapper.unmount();
  });

  // A response belonging to a task the panel no longer shows must not decide
  // the state the panel now displays.
  it("does not let a stale task response clear the current failure state", async () => {
    vi.useFakeTimers();
    let resolveFirst: ((value: unknown) => void) | null = null;
    const refresh = vi.fn()
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockRejectedValue(new Error("current task failed"));
    const { api, taskId, wrapper } = mountRefresh({ refresh });

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    taskId.value = "task-b";
    resolveFirst!(undefined);
    await vi.advanceTimersByTimeAsync(0);

    // The current task's own reload fails, so the notice must appear.
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(api.refreshError.value).toBe(true);

    // A late settle from the previous task must not clear it. The overlap
    // guard holds the next tick until the in-flight request resolves, so the
    // stale answer is observed here rather than raced.
    const callsBefore = refresh.mock.calls.length;
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(refresh.mock.calls.length).toBe(callsBefore + 1);
    expect(api.refreshError.value).toBe(true);
    wrapper.unmount();
  });

  it("stops reloading for a terminal task and resumes when it becomes active again", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const { api, enabled, wrapper } = mountRefresh({ refresh });

    enabled.value = false;
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS * 3);
    expect(refresh).not.toHaveBeenCalled();

    enabled.value = true;
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(api.isRefreshing.value).toBe(false);
    wrapper.unmount();
  });

  it("releases the timer on unmount so a collapsed panel stops polling", async () => {
    vi.useFakeTimers();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const { wrapper } = mountRefresh({ refresh });

    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS);
    expect(refresh).toHaveBeenCalledTimes(1);

    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(ITEM_REFRESH_INTERVAL_MS * 5);
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});