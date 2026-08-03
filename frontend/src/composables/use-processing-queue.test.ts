import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { useProcessingQueue } from "./use-processing-queue";

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const cancelTask = vi.spyOn(apiClient, "cancelTask");

const failedTask: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  status: "failed",
  group_path: "research",
  source_key: null,
  stage: "indexing",
  waiting_reason: null,
  dependency_key: null,
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 1, cancelled: 0 },
  failure_stage: "indexing",
  error_summary: "Embedding failed",
  eta_seconds: null,
  created_at: "2026-07-20T00:00:00Z",
  started_at: "2026-07-20T00:01:00Z",
  finished_at: "2026-07-20T00:02:00Z",
  updated_at: "2026-07-20T00:02:00Z",
};

const waitingTask: TaskResponse = {
  ...failedTask,
  task_id: "waiting-task-id",
  status: "waiting",
  stage: "docling",
  waiting_reason: "dependency",
  dependency_key: "docling",
  progress: { total: 1, queued: 0, running: 0, waiting: 1, succeeded: 0, failed: 0, cancelled: 0 },
  failure_stage: null,
  error_summary: null,
  finished_at: null,
};

function page(items: TaskResponse[] = [failedTask]) {
  return {
    items,
    pagination: { page: 1, page_size: 25, total: items.length, total_pages: items.length ? 1 : 0 },
  };
}

describe("useProcessingQueue", () => {
  beforeEach(() => {
    listTasks.mockReset().mockResolvedValue(page() as never);
    retryTask.mockReset().mockResolvedValue({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
  });

  function mountState() {
    let state!: ReturnType<typeof useProcessingQueue>;
    const wrapper = mount(defineComponent({
      setup() {
        state = useProcessingQueue({ t: (key) => key });
        return {};
      },
      template: "<div />",
    }), { global: { plugins: [testNuxtUiPlugin, createTestI18n()] } });
    return { state, wrapper };
  }

  it("loads tasks and applies status, stage, search, and pagination filters", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(listTasks).toHaveBeenCalledOnce();
    expect(listTasks).toHaveBeenLastCalledWith(
      {
        page: 1,
        pageSize: 25,
        query: "",
        kind: null,
        status: null,
        stage: null,
        waitingReason: null,
      },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.setStatusFilter("waiting");
    state.setStageFilter("docling");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "waiting", stage: "docling" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.searchInput.value = "embedding";
    state.submitSearch();
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: "embedding" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.changePage(2);
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ page: 2, status: "waiting", stage: "docling" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("retries a failed task through the unified task endpoint", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([waitingTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.retryTask(failedTask);

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.items.value[0].status).toBe("waiting");
    wrapper.unmount();
  });

  it("cancels waiting tasks through the unified task endpoint", async () => {
    listTasks
      .mockResolvedValueOnce(page([waitingTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.cancelTask(waitingTask);

    expect(cancelTask).toHaveBeenCalledWith("waiting-task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("retries all visible failed tasks and refreshes once", async () => {
    const secondFailedTask = { ...failedTask, task_id: "task-id-2" };
    listTasks
      .mockResolvedValueOnce(page([failedTask, secondFailedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.retryAllFailed();

    expect(retryTask).toHaveBeenCalledTimes(2);
    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(retryTask).toHaveBeenCalledWith("task-id-2");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
});
