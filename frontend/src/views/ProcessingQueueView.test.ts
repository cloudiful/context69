import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as nuxtUiComposables from "@nuxt/ui/composables";
import { createMemoryHistory, createRouter } from "vue-router";

import ProcessingQueueView from "./ProcessingQueueView.vue";
import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const cancelTask = vi.spyOn(apiClient, "cancelTask");
const useOverlay = vi.spyOn(nuxtUiComposables, "useOverlay");
const addToast = vi.fn();
const useToast = vi.spyOn(nuxtUiComposables, "useToast");

const row: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  status: "waiting",
  group_path: "research",
  source_key: null,
  stage: "docling",
  waiting_reason: "dependency",
  dependency_key: "docling",
  progress: { total: 1, queued: 0, running: 0, waiting: 1, succeeded: 0, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:01:00Z",
  finished_at: null,
  updated_at: "2026-07-20T00:02:00Z",
};

const failedRow: TaskResponse = {
  ...row,
  status: "failed",
  waiting_reason: null,
  dependency_key: null,
  failure_stage: "indexing",
  error_summary: "Qdrant unavailable",
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 0 },
  finished_at: "2026-07-20T00:02:00Z",
};

function response(items: TaskResponse[]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

async function mountQueue() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/processing-queue", name: "processing-queue", component: { template: "<div />" } }],
  });
  await router.push("/processing-queue");
  await router.isReady();
  return mount(ProcessingQueueView, {
    global: { plugins: [testNuxtUiPlugin, createTestI18n("en"), router] },
  });
}

describe("ProcessingQueueView", () => {
  beforeEach(() => {
    listTasks.mockReset().mockResolvedValue(response([row]) as never);
    retryTask.mockReset().mockResolvedValue({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
    useOverlay.mockReset().mockReturnValue({
      create: () => ({ open: async () => true }),
    } as never);
    addToast.mockReset();
    useToast.mockReset().mockReturnValue({ add: addToast } as never);
  });

  it("shows waiting stage and dependency reason", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    expect(wrapper.text()).toContain("Docling");
    expect(wrapper.text()).toContain("docling");
    expect(wrapper.text()).toContain("Waiting");
    wrapper.unmount();
  });

  it("retries a failed task through the unified endpoint", async () => {
    listTasks
      .mockResolvedValueOnce(response([failedRow]) as never)
      .mockResolvedValueOnce(response([row]) as never);
    const wrapper = await mountQueue();
    await flushPromises();

    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry"));
    expect(retryButton).toBeDefined();
    await retryButton!.trigger("click");
    await flushPromises();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("shows retry for a cancelled task with failed items", async () => {
    const cancelledFailedRow: TaskResponse = {
      ...failedRow,
      task_id: "cancelled-task-id",
      status: "cancelled",
      progress: { total: 2, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 1 },
    };
    listTasks.mockResolvedValue(response([cancelledFailedRow]) as never);

    const wrapper = await mountQueue();
    await flushPromises();

    const retryButton = wrapper.findAll("button").find((button) => button.text().includes("Retry"));
    expect(retryButton).toBeDefined();
    wrapper.unmount();
  });

  it("cancels a waiting task through the unified endpoint", async () => {
    const wrapper = await mountQueue();
    await flushPromises();

    const cancelButton = wrapper.findAll("button").find((button) => button.attributes("aria-label")?.includes("Cancel"));
    expect(cancelButton).toBeDefined();
    await cancelButton!.trigger("click");
    await flushPromises();

    expect(cancelTask).toHaveBeenCalledWith("task-id");
    wrapper.unmount();
  });
});
