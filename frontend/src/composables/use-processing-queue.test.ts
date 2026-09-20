import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient, type TaskResponse } from "../services/api";
import { createTestI18n } from "../test-utils/i18n";
import { testNuxtUiPlugin } from "../test-utils/nuxt-ui";
import { useProcessingQueue } from "./use-processing-queue";

const listTasks = vi.spyOn(apiClient, "listTasks");
const retryTask = vi.spyOn(apiClient, "retryTask");
const rerunTask = vi.spyOn(apiClient, "rerunTask");
const cancelTask = vi.spyOn(apiClient, "cancelTask");
const clearTaskHistory = vi.spyOn(apiClient, "clearTaskHistory");

const failedTask: TaskResponse = {
  task_id: "task-id",
  kind: "file_batch",
  origin: "manual",
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

const cancelledTask: TaskResponse = {
  ...failedTask,
  task_id: "cancelled-task-id",
  status: "cancelled",
  progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 0, failed: 0, cancelled: 1 },
  failure_stage: null,
  error_summary: null,
  finished_at: "2026-07-20T00:03:00Z",
};

const waitingTask: TaskResponse = {
  ...failedTask,
  task_id: "waiting-task-id",
  status: "waiting",
  stage: "processing",
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
    rerunTask.mockReset().mockResolvedValue({ task: { task_id: "new-task-id", item_ids: [] } } as never);
    cancelTask.mockReset().mockResolvedValue(undefined);
    clearTaskHistory.mockReset().mockResolvedValue({ deleted_count: 3 } as never);
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
         view: "processing",
        stage: null,
        waitingReason: null,
        dependencyKey: null,
      },
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.setStatusFilter("waiting");
    state.setStageFilter("processing");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "waiting", stage: "processing" }),
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
      expect.objectContaining({ page: 2, status: "waiting", stage: "processing" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("forwards the dependency filter as dependency_key on listTasks", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setDependencyKeyFilter("qdrant");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ dependencyKey: "qdrant" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    state.setDependencyKeyFilter(null);
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ dependencyKey: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("retries a failed task through /retry", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([waitingTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(failedTask);

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(rerunTask).not.toHaveBeenCalled();
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.items.value[0].status).toBe("waiting");
    wrapper.unmount();
  });

  it("resubmits a cancelled task through /rerun", async () => {
    listTasks
      .mockResolvedValueOnce(page([cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(cancelledTask);

    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(retryTask).not.toHaveBeenCalled();
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("drops a narrowing status filter after a single rerun so the new task is visible", async () => {
    listTasks
      .mockResolvedValueOnce(page([cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setStatusFilter("cancelled");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: "cancelled" }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );

    await state.recoverTask(cancelledTask);

    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(state.statusFilter.value).toBeNull();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("keeps the status filter when a single retry creates no new task", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([failedTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setStatusFilter("failed");
    await flushPromises();

    await state.recoverTask(failedTask);

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(state.statusFilter.value).toBe("failed");
    wrapper.unmount();
  });

  it("does not count a waiting task as recoverable while its dependency is unavailable", async () => {
    listTasks.mockResolvedValueOnce(page([waitingTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.isRecoverableTask(waitingTask)).toBe(false);
    expect(state.recoverableCount.value).toBe(0);

    await state.recoverTask(waitingTask);

    expect(retryTask).not.toHaveBeenCalled();
    expect(rerunTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("refreshes and reports a failed retry without a task-level recovery endpoint", async () => {
    const { ApiError } = await import("../services/api/api-core");
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    retryTask.mockRejectedValueOnce(new ApiError("no retryable failed items", 409));
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverTask(failedTask);

    expect(retryTask).toHaveBeenCalledTimes(1);
    expect(listTasks).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("rejects a duplicate task-scoped action while the same task is in flight", async () => {
    let resolveRetry: (() => void) | null = null;
    retryTask.mockReset().mockImplementationOnce(() => new Promise((resolve) => {
      resolveRetry = () => resolve({ task: { task_id: "task-id", item_ids: [] }, retried_items: 1 } as never);
    }));
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    const inFlight = state.recoverTask(failedTask);
    await state.recoverTask(failedTask);

    expect(retryTask).toHaveBeenCalledTimes(1);

    resolveRetry!();
    await inFlight;
    wrapper.unmount();
  });

  it("counts failed and cancelled tasks as recoverable, ignoring succeeded and active ones", async () => {
    const succeededTask: TaskResponse = {
      ...failedTask,
      task_id: "succeeded-task-id",
      status: "succeeded",
      progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
    };
    listTasks.mockResolvedValueOnce(page([failedTask, cancelledTask, waitingTask, succeededTask]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.isRecoverableTask(failedTask)).toBe(true);
    expect(state.isRecoverableTask(cancelledTask)).toBe(true);
    expect(state.isRecoverableTask(waitingTask)).toBe(false);
    expect(state.isRecoverableTask(succeededTask)).toBe(false);
    expect(state.recoverableCount.value).toBe(2);
    expect(state.failedCount.value).toBe(1);
    expect(state.cancelledCount.value).toBe(1);
    expect(state.activeCount.value).toBe(1);
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

  it("recovers every visible failed and cancelled task and refreshes once", async () => {
    const secondFailedTask = { ...failedTask, task_id: "task-id-2" };
    listTasks
      .mockResolvedValueOnce(page([failedTask, secondFailedTask, cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(state.recoverableCount.value).toBe(3);
    await state.recoverAll();

    expect(retryTask).toHaveBeenCalledTimes(2);
    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(retryTask).toHaveBeenCalledWith("task-id-2");
    expect(rerunTask).toHaveBeenCalledTimes(1);
    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("drops a narrowing status filter after bulk rerun creates new tasks", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask, cancelledTask]) as never)
      .mockResolvedValueOnce(page([failedTask, cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setStatusFilter("failed");
    await flushPromises();

    await state.recoverAll();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(state.statusFilter.value).toBeNull();
    expect(listTasks).toHaveBeenLastCalledWith(
      expect.objectContaining({ status: null }),
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    wrapper.unmount();
  });

  it("keeps the status filter after bulk recovery with no rerun", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([failedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    state.setStatusFilter("failed");
    await flushPromises();

    await state.recoverAll();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(rerunTask).not.toHaveBeenCalled();
    expect(state.statusFilter.value).toBe("failed");
    wrapper.unmount();
  });

  it("excludes waiting tasks from bulk recovery and reruns only cancelled ones", async () => {
    listTasks
      .mockResolvedValueOnce(page([waitingTask, cancelledTask, failedTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    const { state, wrapper } = mountState();

    await flushPromises();

    expect(state.isRecoverableTask(waitingTask)).toBe(false);
    expect(state.recoverableCount.value).toBe(2);

    await state.recoverAll();

    expect(rerunTask).toHaveBeenCalledTimes(1);
    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(retryTask).toHaveBeenCalledTimes(1);
    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(retryTask).not.toHaveBeenCalledWith("waiting-task-id");
    wrapper.unmount();
  });

  it("does not trigger bulk recovery when only waiting tasks are visible", async () => {
    listTasks.mockResolvedValueOnce(page([waitingTask]) as never);
    const { state, wrapper } = mountState();

    await flushPromises();

    expect(state.recoverableCount.value).toBe(0);

    await state.recoverAll();

    expect(retryTask).not.toHaveBeenCalled();
    expect(rerunTask).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("keeps refreshing and reporting when one recovery fails", async () => {
    listTasks
      .mockResolvedValueOnce(page([failedTask, cancelledTask]) as never)
      .mockResolvedValueOnce(page([]) as never);
    retryTask.mockRejectedValueOnce(new Error("no retryable failed items"));
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.recoverAll();

    expect(retryTask).toHaveBeenCalledWith("task-id");
    expect(rerunTask).toHaveBeenCalledWith("cancelled-task-id");
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("switches typed list views without status guessing and narrows processing by status", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing", status: null }),
      expect.anything(),
    );

    state.setListView({ view: "completed" });
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "completed", status: null }),
      expect.anything(),
    );

    state.setListView({ view: "trash" });
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "trash", status: null }),
      expect.anything(),
    );

    state.setListView({ view: "processing" });
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing", status: null }),
      expect.anything(),
    );

    // A user-selected status within processing narrows the same view.
    state.setStatusFilter("failed");
    await flushPromises();
    expect(listTasks).toHaveBeenLastCalledWith(
       expect.objectContaining({ view: "processing", status: "failed" }),
      expect.anything(),
    );
    wrapper.unmount();
  });

  it("clears completed history through the user-scoped endpoint and refreshes", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.clearHistory("completed");

    expect(clearTaskHistory).toHaveBeenCalledWith({ view: "completed" });
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.clearAction.value).toBeNull();
    wrapper.unmount();
  });

  it("clears trashed history through the user-scoped endpoint and refreshes", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.clearHistory("trash");

    expect(clearTaskHistory).toHaveBeenCalledWith({ view: "trash" });
    expect(listTasks).toHaveBeenCalledTimes(2);
    expect(state.clearAction.value).toBeNull();
    wrapper.unmount();
  });

  it("clears the completed view with an explicit deleted count", async () => {
    clearTaskHistory.mockResolvedValueOnce({ deleted_count: 0 } as never);
    const { state, wrapper } = mountState();
    await flushPromises();

    await state.clearHistory("completed");

    expect(clearTaskHistory).toHaveBeenCalledWith({ view: "completed" });
    expect(listTasks).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("does not clear while a bulk recover or cancel is in flight", async () => {
    const { state, wrapper } = mountState();
    await flushPromises();

    state.bulkAction.value = "recover";
    await state.clearHistory("completed");

    expect(clearTaskHistory).not.toHaveBeenCalled();
    state.bulkAction.value = null;
    wrapper.unmount();
  });

  it("does not bulk recover while a history clear is in flight", async () => {
    let resolveClear: ((value: { deleted_count: number }) => void) | null = null;
    clearTaskHistory.mockReset().mockImplementationOnce(() => new Promise((resolve) => {
      resolveClear = resolve as (value: { deleted_count: number }) => void;
    }));
    const { state, wrapper } = mountState();
    await flushPromises();

    const clearing = state.clearHistory("trash");
    expect(state.clearAction.value).toBe("trash");

    await state.recoverAll();
    expect(retryTask).not.toHaveBeenCalled();
    expect(rerunTask).not.toHaveBeenCalled();

    resolveClear!({ deleted_count: 1 });
    await clearing;
    expect(clearTaskHistory).toHaveBeenCalledWith({ view: "trash" });
    wrapper.unmount();
  });
});
