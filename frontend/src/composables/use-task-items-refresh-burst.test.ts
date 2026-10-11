import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, ref } from "vue";

import { useTaskItemsRefresh, type UseTaskItemsRefreshOptions } from "./use-task-items-refresh";

function mountRefresh() {
  const taskId = ref("task-a");
  const enabled = ref(true);
  let api!: ReturnType<typeof useTaskItemsRefresh>;
  const wrapper = mount(defineComponent({
    setup() {
      const options: UseTaskItemsRefreshOptions = {
        taskId,
        enabled,
        refresh: async () => undefined,
      };
      api = useTaskItemsRefresh(options);
      return () => h("div");
    },
  }));
  return { api, taskId, enabled, wrapper };
}

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

afterEach(() => vi.useRealTimers());

describe("automatic task-item refresh bursts", () => {
  it("keeps one latest trailing refresh for a 100-trigger burst", async () => {
    vi.useFakeTimers();
    const { api, wrapper } = mountRefresh();
    const requests: number[] = [];
    const pending: Array<ReturnType<typeof deferred>> = [];
    let active = 0;
    let maximumActive = 0;
    const request = (revision: number) => {
      requests.push(revision);
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      const next = deferred();
      pending.push(next);
      return next.promise.finally(() => { active -= 1; });
    };

    api.scheduleAutomaticRefresh(() => request(0));
    await flushPromises();
    for (let revision = 1; revision <= 100; revision += 1) {
      api.scheduleAutomaticRefresh(() => request(revision));
    }
    expect(requests).toEqual([0]);
    expect(active).toBe(1);

    pending[0]!.resolve();
    await flushPromises();
    expect(requests).toEqual([0, 100]);
    expect(maximumActive).toBe(1);
    expect(api.isRefreshing.value).toBe(true);

    pending[1]!.resolve();
    await flushPromises();
    expect(requests).toEqual([0, 100]);
    expect(api.isRefreshing.value).toBe(false);
    expect(api.refreshError.value).toBe(false);
    wrapper.unmount();
  });

  it("drops the trailing request when the query generation is invalidated", async () => {
    vi.useFakeTimers();
    const { api, wrapper } = mountRefresh();
    const active = deferred();
    const request = vi.fn(() => active.promise);
    api.scheduleAutomaticRefresh(request);
    await flushPromises();
    api.scheduleAutomaticRefresh(request);
    api.invalidateAutomaticRefresh();

    active.resolve();
    await flushPromises();
    expect(request).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("lets queued explicit work run before the trailing automatic refresh", async () => {
    vi.useFakeTimers();
    const { api, wrapper } = mountRefresh();
    const order: string[] = [];
    const automatic = deferred();
    const explicit = deferred();
    api.scheduleAutomaticRefresh(() => {
      order.push("automatic");
      return automatic.promise;
    });
    await flushPromises();
    api.scheduleAutomaticRefresh(async () => { order.push("trailing"); });
    const explicitRun = api.runExclusive(() => {
      order.push("explicit");
      return explicit.promise;
    });

    automatic.resolve();
    await flushPromises();
    expect(order).toEqual(["automatic", "explicit"]);
    explicit.resolve();
    await explicitRun;
    await flushPromises();
    expect(order).toEqual(["automatic", "explicit", "trailing"]);
    wrapper.unmount();
  });
});
