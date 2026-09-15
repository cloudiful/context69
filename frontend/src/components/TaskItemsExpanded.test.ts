import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import TaskItemsExpanded from "./TaskItemsExpanded.vue";
import { apiClient, type TaskItemResponse, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const getTaskItems = vi.spyOn(apiClient, "getTaskItems");

function task(total = 2781): TaskResponse {
  return {
    task_id: "task-id",
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: "research",
    source_key: null,
    stage: "indexing",
    waiting_reason: null,
    dependency_key: null,
    progress: { total, queued: 0, running: 1, waiting: 0, succeeded: 0, failed: 0, cancelled: 0 },
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-07-20T00:01:00Z",
    started_at: "2026-07-20T00:01:00Z",
    finished_at: null,
    updated_at: "2026-07-20T00:02:00Z",
  };
}

function item(itemId: string, status: TaskItemResponse["status"] = "running"): TaskItemResponse {
  return {
    item_id: itemId,
    status,
    stage: "indexing",
    ordinal: 0,
    attempt_count: 1,
    retryable: false,
    error_message: null,
    created_at: "2026-07-20T00:01:00Z",
    updated_at: "2026-07-20T00:02:00Z",
  } as TaskItemResponse;
}

function mountExpanded(entry: TaskResponse = task()) {
  return mount(TaskItemsExpanded, {
    props: { task: entry, isAdmin: false, isActing: false },
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en")] },
  });
}

describe("TaskItemsExpanded", () => {
  beforeEach(() => {
    getTaskItems.mockReset();
  });

  it("loads the first page and shows the shown/total count", async () => {
    getTaskItems.mockResolvedValue({ items: [item("item-1")], next_cursor: "cursor-1" } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(1);
    expect(getTaskItems).toHaveBeenCalledWith("task-id", expect.objectContaining({ limit: 100 }));
    expect(wrapper.find('[data-testid="task-items-count"]').text()).toContain("1");
    expect(wrapper.find('[data-testid="task-items-count"]').text()).toContain("2781");
    expect(wrapper.text()).toContain("item-1");
    expect(wrapper.find('[data-testid="task-items-load-more"]').exists()).toBe(true);
    wrapper.unmount();
  });

  it("follows next_cursor on load more until the cursor is null", async () => {
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("item-2")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    await wrapper.find('[data-testid="task-items-load-more"]').trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(getTaskItems).toHaveBeenLastCalledWith(
      "task-id",
      expect.objectContaining({ limit: 100, cursor: "cursor-1" }),
    );
    expect(wrapper.text()).toContain("item-1");
    expect(wrapper.text()).toContain("item-2");
    expect(wrapper.find('[data-testid="task-items-count"]').text()).toContain("2");
    expect(wrapper.find('[data-testid="task-items-load-more"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("resets the cursor and sends the status filter when the filter changes", async () => {
    getTaskItems
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: "cursor-1" } as never)
      .mockResolvedValueOnce({ items: [item("failed-1", "failed")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    const trigger = wrapper.findComponent('[data-testid="task-items-filter"]') as unknown as {
      exists(): boolean;
      vm: { $parent: unknown };
    };
    expect(trigger.exists()).toBe(true);
    // The testid falls through to the inner Reka SelectTrigger; the
    // `update:model-value` listener lives on the USelect ancestor (chain:
    // SelectTrigger -> PopperRoot -> SelectRoot -> USelect-as-Select).
    let selectVm = trigger.vm.$parent as unknown as {
      $options?: { __name?: string };
      $parent?: unknown;
      $emit?: (event: string, value: unknown) => void;
    } | null;
    while (selectVm && selectVm.$options?.__name !== "Select") {
      selectVm = selectVm.$parent as typeof selectVm;
    }
    expect(selectVm?.$emit).toBeDefined();
    selectVm!.$emit!("update:model-value", "failed");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    // Cursor is scoped to the filter: a filter change restarts without one.
    expect(getTaskItems).toHaveBeenLastCalledWith(
      "task-id",
      expect.objectContaining({ limit: 100, status: "failed" }),
    );
    expect(wrapper.text()).toContain("failed-1");
    expect(wrapper.text()).not.toContain("item-1");
    wrapper.unmount();
  });

  it("exposes a retry-load button when the first page fails and reuses getTaskItems", async () => {
    getTaskItems
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce({ items: [item("item-1")], next_cursor: null } as never);
    const wrapper = mountExpanded();
    await flushPromises();

    expect(wrapper.text()).toContain("Failed to load task items");
    const retryLoad = wrapper.findAll("button").find((button) => button.attributes("aria-label") === "Retry loading items");
    expect(retryLoad).toBeDefined();
    await retryLoad!.trigger("click");
    await flushPromises();

    expect(getTaskItems).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("item-1");
    wrapper.unmount();
  });
});
