import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ProcessingQueueTable from "./ProcessingQueueTable.vue";
import TaskItemsExpanded from "../TaskItemsExpanded.vue";
import { apiClient, type TaskResponse } from "../../services/api";
import { createTestI18n } from "../../test-utils/i18n";
import { testNuxtUiPlugin } from "../../test-utils/nuxt-ui";

const getTaskItems = vi.spyOn(apiClient, "getTaskItems");

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

function mountTable(items: TaskResponse[]) {
  return mount(ProcessingQueueTable, {
    props: {
      items,
      loading: false,
      view: "processing",
      sort: null,
      statusFilter: null,
      kindFilter: null,
      stageFilter: null,
      waitingReasonFilter: null,
      dependencyKeyFilter: null,
      isActing: () => false,
      isRecoverableTask: (entry: TaskResponse) => entry.status === "failed" || entry.status === "cancelled",
    },
    global: { plugins: [testNuxtUiPlugin, createTestI18n()] },
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  getTaskItems.mockReset();
  getTaskItems.mockResolvedValue({ items: [], next_cursor: null } as never);
});
afterEach(() => vi.useRealTimers());

describe("ProcessingQueueTable identity and updates", () => {
  it("keeps an expanded task through reorder and prunes it when it leaves the page", async () => {
    const first = task("task-a");
    const second = task("task-b", "queued");
    const wrapper = mountTable([first, second]);
    try {
      await wrapper.findAll('[aria-label="Expand task items"]')[0]!.trigger("click");
      await flushPromises();
      expect(wrapper.findAllComponents(TaskItemsExpanded).map((component) =>
        (component.props("task") as TaskResponse).task_id,
      )).toEqual(["task-a"]);

      await wrapper.setProps({ items: [second, first] });
      await flushPromises();
      expect(wrapper.findAllComponents(TaskItemsExpanded).map((component) =>
        (component.props("task") as TaskResponse).task_id,
      )).toEqual(["task-a"]);

      await wrapper.setProps({ items: [second] });
      await flushPromises();
      expect(wrapper.findAllComponents(TaskItemsExpanded)).toHaveLength(0);
      await wrapper.setProps({ items: [first, second] });
      await flushPromises();
      expect(wrapper.findAllComponents(TaskItemsExpanded)).toHaveLength(0);
      expect(wrapper.text()).not.toContain("//");
    } finally {
      wrapper.unmount();
    }
  });

  it("shows immutable task replacements with shallow table watching", async () => {
    const initial = task("task-a");
    const wrapper = mountTable([initial]);
    try {
      expect(wrapper.find("tbody tr").text()).toContain("research");
      await wrapper.setProps({ items: [{ ...initial, group_path: "archive" }] });
      await flushPromises();
      expect(wrapper.find("tbody tr").text()).toContain("archive");
      expect(wrapper.find("tbody tr").text()).not.toContain("research");
    } finally {
      wrapper.unmount();
    }
  });
});
