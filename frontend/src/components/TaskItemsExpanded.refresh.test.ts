import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import TaskItemsExpanded from "./TaskItemsExpanded.vue";
import { apiClient, type TaskItemResponse, type TaskItemStatus, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const getTaskItems = vi.spyOn(apiClient, "getTaskItems");

function task(updatedAt = "2026-07-20T00:02:00Z"): TaskResponse {
  return {
    task_id: "task-a",
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: "research",
    source_key: null,
    stage: "processing",
    waiting_reason: null,
    dependency_key: null,
    progress: { total: 3, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-07-20T00:00:00Z",
    updated_at: updatedAt,
  };
}

function item(itemId: string, fileName: string, status: TaskItemResponse["status"] = "running"): TaskItemResponse {
  return {
    item_id: itemId,
    status,
    stage: "processing",
    ordinal: 0,
    attempt_count: 1,
    retryable: false,
    error_message: null,
    created_at: "2026-07-20T00:01:00Z",
    updated_at: "2026-07-20T00:02:00Z",
    progress: null,
    file_name: fileName,
  } as TaskItemResponse;
}

function mountExpanded(entry = task()) {
  return mount(TaskItemsExpanded, {
    props: { task: entry, isActing: false },
    global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
  });
}

function setStatusFilter(wrapper: ReturnType<typeof mountExpanded>, status: TaskItemStatus) {
  const filter = wrapper.findComponent('[data-testid="task-items-filter"]') as unknown as {
    exists(): boolean;
    vm: { $parent: unknown };
  };
  expect(filter.exists()).toBe(true);
  let selectVm = filter.vm.$parent as {
    $options?: { __name?: string };
    $parent?: unknown;
    $emit?: (event: string, value: unknown) => void;
  } | null;
  while (selectVm && selectVm.$options?.__name !== "Select") selectVm = selectVm.$parent as typeof selectVm;
  selectVm!.$emit!("update:model-value", status);
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  vi.useFakeTimers();
  getTaskItems.mockReset();
});
afterEach(() => vi.useRealTimers());

describe("expanded task automatic refresh", () => {
  it("coalesces 100 timestamp changes into one trailing full-extent refresh", async () => {
    type ItemPage = Awaited<ReturnType<typeof apiClient.getTaskItems>>;
    const firstRefreshPage = deferred<ItemPage>();
    const trailingPage = deferred<ItemPage>();
    getTaskItems
      .mockResolvedValueOnce({ items: [item("initial-1", "initial-1"), item("initial-2", "initial-2")], next_cursor: "initial-next" } as never)
      .mockResolvedValueOnce({ items: [item("initial-3", "initial-3")], next_cursor: null } as never)
      .mockImplementationOnce(() => firstRefreshPage.promise)
      .mockResolvedValueOnce({ items: [item("middle-3", "middle-3")], next_cursor: null } as never)
      .mockResolvedValueOnce({ items: [item("latest-1", "latest-1"), item("latest-2", "latest-2")], next_cursor: "latest-next" } as never)
      .mockImplementationOnce(() => trailingPage.promise);

    const entry = task();
    const wrapper = mountExpanded(entry);
    try {
      await flushPromises();
      await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
      await flushPromises();
      expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(3);

      await wrapper.setProps({ task: { ...entry, updated_at: "2026-07-20T00:02:01Z" } });
      await flushPromises();
      expect(getTaskItems).toHaveBeenCalledTimes(3);
      for (let revision = 2; revision <= 100; revision += 1) {
        await wrapper.setProps({
          task: { ...entry, updated_at: new Date(Date.parse(entry.updated_at!) + revision * 1000).toISOString() },
        });
      }
      expect(getTaskItems).toHaveBeenCalledTimes(3);
      expect(wrapper.text()).toContain("initial-3");

      firstRefreshPage.resolve({ items: [item("middle-1", "middle-1"), item("middle-2", "middle-2")], next_cursor: "middle-next" } as never);
      await flushPromises();
      expect(getTaskItems).toHaveBeenCalledTimes(6);
      expect(getTaskItems).toHaveBeenNthCalledWith(4, "task-a", expect.objectContaining({ cursor: "middle-next" }));
      expect(getTaskItems).toHaveBeenNthCalledWith(6, "task-a", expect.objectContaining({ cursor: "latest-next" }));
      expect(wrapper.text()).toContain("middle-3");
      expect(wrapper.text()).not.toContain("latest-1");

      trailingPage.resolve({ items: [item("latest-3", "latest-3")], next_cursor: null } as never);
      await flushPromises();
      expect(wrapper.findAll('[data-testid="task-item-row"]')).toHaveLength(3);
      expect(wrapper.text()).toContain("latest-1");
      expect(wrapper.text()).toContain("latest-3");
      expect(wrapper.text()).not.toContain("middle-1");
      expect(wrapper.find('[data-testid="task-items-load-more"]').exists()).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it("clears a pending trailing refresh when the item filter changes", async () => {
    type ItemPage = Awaited<ReturnType<typeof apiClient.getTaskItems>>;
    const automaticPage = deferred<ItemPage>();
    getTaskItems
      .mockResolvedValueOnce({ items: [item("initial", "initial")], next_cursor: null } as never)
      .mockImplementationOnce(() => automaticPage.promise)
      .mockResolvedValueOnce({ items: [item("filtered", "filtered", "failed")], next_cursor: null } as never);
    const entry = task();
    const wrapper = mountExpanded(entry);
    try {
      await flushPromises();
      await wrapper.setProps({ task: { ...entry, updated_at: "2026-07-20T00:02:01Z" } });
      await flushPromises();
      await wrapper.setProps({ task: { ...entry, updated_at: "2026-07-20T00:02:02Z" } });
      setStatusFilter(wrapper, "failed");
      await flushPromises();

      automaticPage.resolve({ items: [item("stale", "stale")], next_cursor: null } as never);
      await flushPromises();

      expect(getTaskItems).toHaveBeenCalledTimes(3);
      expect(getTaskItems).toHaveBeenLastCalledWith("task-a", expect.objectContaining({ status: "failed" }));
      expect(wrapper.text()).toContain("filtered");
      expect(wrapper.text()).not.toContain("stale");
    } finally {
      wrapper.unmount();
    }
  });
});
